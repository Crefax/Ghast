//! Events as a stream, `wait_for`, and chat helpers.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use ghast_client::inventory::ClickResult;
use ghast_client::{BlockPos, ChatMessage, GameMode, Health, TeleportEvent};

use crate::executor::lock;
use crate::find::EntityInfo;
use crate::{Bot, Error};

/// Something that happened, as delivered to [`Bot::events`] subscribers (in the order the
/// tick saw them).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// A chat message ([`ghast_client::Interests::CHAT`], on by default in [`crate::Ext`]).
    Chat(ChatMessage),
    /// The server teleported / set back the player.
    Teleport(TeleportEvent),
    /// A join game packet (first join or a proxy server switch).
    Joined,
    /// Respawned or changed dimension.
    Respawned,
    /// The player died.
    Died,
    /// Health, food or saturation changed (the new values).
    Health(Health),
    /// The game mode changed.
    GameMode(GameMode),
    /// A container window opened (its id).
    WindowOpened(i32),
    /// The open window closed.
    WindowClosed,
    /// The player inventory changed.
    InventoryChanged,
    /// A click's confirm (or timeout).
    Click(ClickResult),
    /// A block changed ([`ghast_client::Interests::BLOCK_UPDATES`]): the version's raw state.
    BlockUpdate {
        /// Position.
        pos: BlockPos,
        /// Raw block state (see [`crate::Block`]).
        raw: u32,
    },
    /// An entity appeared ([`ghast_client::Interests::ENTITIES`]; entities that existed when
    /// the subscription started are not reported).
    EntitySpawned(EntityInfo),
    /// An entity is gone (despawned, out of range, killed).
    EntityGone(i32),
}

pub(crate) struct SubState {
    pub(crate) queue: VecDeque<Event>,
    pub(crate) waker: Option<Waker>,
}

/// A subscription to the bot's events ([`Bot::events`]): events from the tick after it
/// was made, buffered until read. Dropping it unsubscribes.
pub struct Events {
    bot: Bot,
    session: u64,
    state: Arc<Mutex<SubState>>,
}

impl std::fmt::Debug for Events {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Events")
            .field("buffered", &lock(&self.state).queue.len())
            .finish()
    }
}

impl Events {
    /// The next event; `None` once the session ended.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) {
    /// let mut events = bot.events();
    /// while let Some(ev) = events.recv().await {
    ///     println!("{ev:?}");
    /// }
    /// # }
    /// ```
    pub fn recv(&mut self) -> NextEvent<'_> {
        NextEvent { events: self }
    }

    /// An event that is already buffered, without waiting.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) {
    /// let mut events = bot.events();
    /// bot.next_tick().await;
    /// while let Some(ev) = events.try_next() { /* ... */ }
    /// # }
    /// ```
    pub fn try_next(&mut self) -> Option<Event> {
        lock(&self.state).queue.pop_front()
    }
}

/// The future of [`Events::recv`].
#[must_use = "futures do nothing unless awaited"]
#[derive(Debug)]
pub struct NextEvent<'a> {
    events: &'a mut Events,
}

impl Future for NextEvent<'_> {
    type Output = Option<Event>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Event>> {
        let ev = &*self.events;
        let mut st = lock(&ev.state);
        if let Some(e) = st.queue.pop_front() {
            return Poll::Ready(Some(e));
        }
        let s = ev.bot.lock();
        if s.closed.is_some() || s.session != ev.session {
            return Poll::Ready(None);
        }
        st.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Bot {
    /// Subscribes to the bot's events from the next tick on (buffered until read).
    ///
    /// ```no_run
    /// use ghast_ext::Event;
    /// # async fn f(bot: ghast_ext::Bot) {
    /// let mut events = bot.events();
    /// while let Some(ev) = events.recv().await {
    ///     match ev {
    ///         Event::Chat(m) => println!("chat: {}", m.plain()),
    ///         Event::Died => bot.respawn(),
    ///         _ => {}
    ///     }
    /// }
    /// # }
    /// ```
    #[must_use]
    pub fn events(&self) -> Events {
        let state = Arc::new(Mutex::new(SubState {
            queue: VecDeque::new(),
            waker: None,
        }));
        let mut s = self.lock();
        s.subs.push(Arc::downgrade(&state));
        Events {
            bot: self.clone(),
            session: s.session,
            state,
        }
    }

    /// Waits for the first event (from the next tick on) that matches `pred` and returns
    /// it. Fails with [`Error::Disconnected`] when the session ends first.
    ///
    /// ```no_run
    /// use ghast_ext::Event;
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.wait_for(|ev| matches!(ev, Event::Chat(m) if m.plain().contains("hi"))).await?;
    /// bot.chat("hello");
    /// # Ok(()) }
    /// ```
    pub async fn wait_for<F>(&self, mut pred: F) -> Result<Event, Error>
    where
        F: FnMut(&Event) -> bool + Send,
    {
        let mut ev = self.events();
        while let Some(e) = ev.recv().await {
            if pred(&e) {
                return Ok(e);
            }
        }
        Err(Error::Disconnected)
    }

    /// Sends a chat line or a command (`/...`) in this tick (truncated to the version's
    /// limit like vanilla).
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// bot.chat("hello");
    /// bot.chat("/spawn");
    /// # }
    /// ```
    pub fn chat(&self, text: impl Into<String>) {
        let text = text.into();
        self.act(move |ctx| ctx.chat(text));
    }

    /// Waits for a chat message (from the next tick on) that matches `pred`.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let m = bot.wait_for_chat(|m| m.plain().starts_with("<Steve>")).await?;
    /// println!("{}", m.plain());
    /// # Ok(()) }
    /// ```
    pub async fn wait_for_chat<F>(&self, mut pred: F) -> Result<ChatMessage, Error>
    where
        F: FnMut(&ChatMessage) -> bool + Send,
    {
        let mut ev = self.events();
        while let Some(e) = ev.recv().await {
            if let Event::Chat(m) = e
                && pred(&m)
            {
                return Ok(m);
            }
        }
        Err(Error::Disconnected)
    }

    /// Sends `text` and waits up to `ticks` for the first chat reply that matches `pred`
    /// (subscribed before sending, so a fast reply is not missed).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let reply = bot
    ///     .command("/balance", |m| m.plain().contains("Balance"), 100)
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn command<F>(
        &self,
        text: impl Into<String>,
        mut pred: F,
        ticks: u64,
    ) -> Result<ChatMessage, Error>
    where
        F: FnMut(&ChatMessage) -> bool + Send,
    {
        let mut ev = self.events();
        self.chat(text);
        let wait = async {
            while let Some(e) = ev.recv().await {
                if let Event::Chat(m) = e
                    && pred(&m)
                {
                    return Ok(m);
                }
            }
            Err(Error::Disconnected)
        };
        self.timeout(ticks, wait).await?
    }

    /// Asks the server to respawn (only sent while dead).
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// if bot.is_dead() { bot.respawn(); }
    /// # }
    /// ```
    pub fn respawn(&self) {
        self.act(|ctx| {
            ctx.respawn();
        });
    }

    /// Ends the session after this tick's packets (`SessionEnd::Quit(reason)`).
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// bot.disconnect("bye");
    /// # }
    /// ```
    pub fn disconnect(&self, reason: impl Into<String>) {
        let reason = reason.into();
        self.act(move |ctx| ctx.disconnect(reason));
    }
}
