//! Server dialogs (26.1+): the screens a server opens with `show_dialog`, and the answers a
//! vanilla client sends for them.
//!
//! Opt in with [`Interests::DIALOGS`](crate::Interests::DIALOGS). In the play state the shown
//! dialog arrives as [`TickEvents::dialogs`](crate::TickEvents::dialogs) and
//! [`TickContext::dialog`](crate::TickContext::dialog); the behaviour answers with
//! [`TickContext::answer_dialog`](crate::TickContext::answer_dialog). In the configuration state
//! (before the player is in the world, where behaviours do not tick) the client calls
//! [`Behavior::configuration_dialog`](crate::Behavior::configuration_dialog) and sends the answer
//! it returns. Without the interest, play-state dialogs are dropped before they are decoded and a
//! configuration-state dialog is only logged (a warning: a server that waits for the answer would
//! otherwise stall the login without a word).
//!
//! # What the vanilla client does (26.1 and 26.2 behave the same)
//!
//! Established from the vanilla 26.1 / 26.2 client code; written down here in our own words.
//!
//! - **Packets.** `show_dialog` and `clear_dialog` exist in both the configuration and the play
//!   state. In configuration the dialog is always sent inline (unnamed network NBT); in play it is
//!   a registry holder: VarInt 0 and the inline NBT, or the `minecraft:dialog` registry id + 1 (an
//!   entry the configuration's `registry_data` named; entries of a pack the client knows come
//!   without data, and then only the name is known here). `clear_dialog` closes an open dialog
//!   and sends nothing. A new `show_dialog` replaces the open one.
//! - **Data.** A dialog has a `type` (`notice`, `confirmation`, `multi_action`, `dialog_list`,
//!   `server_links`) and common fields: `title`, `external_title`, `body` (one element or a
//!   list: `plain_message` text or an `item`), `inputs`, `can_close_with_escape` (default true),
//!   `pause` (default true; only singleplayer pauses) and `after_action` (`close` default,
//!   `none`, `wait_for_response`). Buttons have a `label`, an optional `tooltip`, a `width`
//!   (default 150) and an optional `action`. A notice has one button (default "gui.ok", no
//!   action), a confirmation `yes` and `no`, a multi-action dialog its `actions` plus an optional
//!   `exit_action`, a dialog list one button per listed dialog (opening it) plus `exit_action`, a
//!   server-links dialog one button per server link (opening the URL; the links come from a
//!   separate packet this client does not keep) plus `exit_action`.
//! - **Inputs** (`key` + control): `text` (`initial` "", `max_length` 32, optional `multiline`),
//!   `boolean` (`initial` false, `on_true` "true", `on_false` "false"), `single_option`
//!   (`options`: ids with an optional display text, at most one `initial`, else the first) and
//!   `number_range` (`start`, `end`, optional `initial` (default the middle) and `step`).
//! - **Button actions.** `run_command` and `dynamic/run_command` (a `$(key)` template filled with
//!   the inputs' text forms) run a command; `custom` and `dynamic/custom` send
//!   `custom_click_action`; `show_dialog` opens another dialog (nothing is sent); `open_url`,
//!   `copy_to_clipboard`, `suggest_command` and `change_page` act on the client only and send
//!   nothing. After the action the dialog closes (`close`), stays (`none`) or is replaced by a
//!   "waiting for response" screen (`wait_for_response`).
//! - **Commands.** The leading `/` is removed. In play, vanilla checks the line against the
//!   server's command tree first: a line that parses, has no signable (`minecraft:message`)
//!   argument and parses without permissions too is sent at once as an unsigned `chat_command`.
//!   Otherwise a confirmation screen asks the player, and nothing is sent unless the player
//!   accepts: "Run command" for parse or permission problems (then the same unsigned
//!   `chat_command`), "Suggest command" / "Copy to clipboard" for a signable one (the player then
//!   sends it from the chat line, signed like any typed command). In the configuration state
//!   commands are not supported: the client logs a warning, sends nothing and the dialog stays
//!   open.
//! - **Text forms of inputs** (templates): text inputs SNBT-escaped without quotes (`"`, `'` and
//!   `\` get a backslash, control characters `\n`-style or `\xHH`), booleans their `on_true` /
//!   `on_false` strings, options their id, numbers as an integer when whole, else Java's
//!   `Float.toString`. A key the template names but no input has becomes "".
//! - **Tag forms of inputs** (`dynamic/custom`): the `additions` compound plus one entry per
//!   input: a string tag (text, unescaped; option id), a byte tag 0 / 1 (boolean) or a float tag
//!   (number). The compound is written in Java `HashMap` iteration order, as vanilla does.
//! - **`custom_click_action`** (serverbound, both states): the id as a string, then the payload as
//!   one network NBT tag (`TAG_End` for none) prefixed by its byte length as a VarInt (at most
//!   65536 bytes). (minecraft-data's 3.119.0 layout, a boolean plus NBT, is not what vanilla
//!   writes; the client writes the vanilla layout itself.)
//! - **Escape** works only when `can_close_with_escape` is true. It runs the dialog's cancel
//!   action (notice: its button's action; confirmation: the `no` action; the button lists: the
//!   `exit_action`'s action) and closes the dialog whatever `after_action` says.
//! - **Numbers** come from a slider: the value is the slider position mapped into the range,
//!   snapped to `step` steps counted from the initial value (never past the ends).
//! - **Typed text** drops `§` and control characters (single-line boxes also drop newlines) and
//!   stops at `max_length` UTF-16 units.
//!
//! Not modelled: the dialog's effect on the rest of the client (a screen is open, so a vanilla
//! player's movement keys and mouse buttons are up meanwhile; the behaviour decides what the bot
//! does while a dialog is open), the multi-line box's line limit, the server links, and dialog
//! tags in a `dialog_list` (`#namespace:tag`).

use std::sync::Arc;

use ghast_protocol::nbt::{AnonNbt, Compound, List, Tag};

/// A dialog the server shows (decoded; see the [module docs](self) for what vanilla does).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Dialog {
    /// The dialog type.
    pub kind: DialogKind,
    /// The title as plain text.
    pub title: String,
    /// `external_title` as plain text (the label of a button that opens this dialog).
    pub external_title: Option<String>,
    /// Escape closes it (and runs [`Dialog::on_cancel`]).
    pub can_close_with_escape: bool,
    /// It pauses a singleplayer game.
    pub pause: bool,
    /// What happens after a button's action.
    pub after_action: AfterAction,
    /// The body elements.
    pub body: Vec<DialogBody>,
    /// The inputs, in order.
    pub inputs: Vec<DialogInput>,
    /// The buttons ([`DialogAnswer::button`] indexes them): a notice's one button, a
    /// confirmation's `yes` and `no`, the actions of a multi-action dialog, one button per
    /// listed dialog of a dialog list. Server-links dialogs list no buttons here.
    pub buttons: Vec<DialogButton>,
    /// The `exit_action` button of the button-list dialogs ([`DialogAnswer::exit`]).
    pub exit: Option<DialogButton>,
    /// The action Escape runs ([`DialogAnswer::close`]).
    pub on_cancel: Option<DialogAction>,
    /// The `minecraft:dialog` registry entry, when the server named one instead of sending the
    /// dialog inline.
    pub registry_name: Option<String>,
    /// The dialog's data as received (`None`: a registry entry without data, e.g. from a pack
    /// the client knows; then only [`Dialog::registry_name`] is known).
    pub nbt: Option<AnonNbt>,
    /// The registry the named dialogs it opens resolve in.
    registry: Option<Arc<DialogRegistry>>,
}

/// The dialog types.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DialogKind {
    /// One button.
    Notice,
    /// `yes` and `no`.
    Confirmation,
    /// A grid of action buttons.
    MultiAction {
        /// Buttons per row.
        columns: i32,
    },
    /// A grid of buttons that open other dialogs.
    DialogList {
        /// Buttons per row.
        columns: i32,
    },
    /// A grid of the server's links (the links themselves are not kept).
    ServerLinks {
        /// Buttons per row.
        columns: i32,
    },
    /// A registry entry whose data the client does not have.
    Unresolved,
    /// A type this client does not know (its id).
    Other(String),
}

/// What happens after a button's action (`after_action`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum AfterAction {
    /// The dialog closes.
    #[default]
    Close,
    /// The dialog stays open.
    None,
    /// A "waiting for response" screen replaces it (the server is expected to answer).
    WaitForResponse,
}

/// A body element.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DialogBody {
    /// `plain_message`: text.
    Text(String),
    /// `item`: an item stack, with an optional description.
    Item {
        /// Item id (e.g. `minecraft:diamond`).
        id: String,
        /// Stack size.
        count: i32,
        /// The description as plain text.
        description: Option<String>,
    },
    /// A body type this client does not know (its id).
    Other(String),
}

/// An input of the dialog.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DialogInput {
    /// The key (template variable and payload entry name).
    pub key: String,
    /// The label as plain text.
    pub label: String,
    /// The control.
    pub kind: InputKind,
}

/// An input control.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum InputKind {
    /// A text box.
    Text {
        /// Initial text.
        initial: String,
        /// Longest text, in UTF-16 units.
        max_length: i32,
        /// A multi-line box.
        multiline: bool,
    },
    /// A checkbox.
    Bool {
        /// Initially checked.
        initial: bool,
        /// Template text when checked.
        on_true: String,
        /// Template text when not checked.
        on_false: String,
    },
    /// A button cycling through options.
    SingleOption {
        /// The options (id, display text) in order.
        options: Vec<(String, String)>,
        /// Index of the initial option.
        initial: usize,
    },
    /// A slider.
    NumberRange {
        /// Value at the slider's left end.
        start: f32,
        /// Value at the slider's right end.
        end: f32,
        /// Initial value (`None`: the middle).
        initial: Option<f32>,
        /// Step the value snaps to, counted from the initial value.
        step: Option<f32>,
    },
    /// A control type this client does not know (its id); it has no value.
    Other(String),
}

/// A button.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DialogButton {
    /// The label as plain text.
    pub label: String,
    /// The tooltip as plain text.
    pub tooltip: Option<String>,
    /// Width in GUI pixels.
    pub width: i32,
    /// What it does (`None`: only the dialog's [`AfterAction`]).
    pub action: Option<DialogAction>,
}

/// A button action.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DialogAction {
    /// `run_command`.
    RunCommand(String),
    /// `dynamic/run_command`: a `$(key)` template filled with the inputs.
    CommandTemplate(String),
    /// `custom`: `custom_click_action` with this id and payload.
    Custom {
        /// Id.
        id: String,
        /// Payload (any tag).
        payload: Option<AnonNbt>,
    },
    /// `dynamic/custom`: `custom_click_action` with the inputs added to `additions`.
    CustomTemplate {
        /// Id.
        id: String,
        /// The compound the inputs are added to.
        additions: Option<AnonNbt>,
    },
    /// `show_dialog`: opens another dialog (inline or a registry name).
    ShowDialog(DialogTarget),
    /// `open_url` (client only).
    OpenUrl(String),
    /// `copy_to_clipboard` (client only).
    CopyToClipboard(String),
    /// `suggest_command` (client only).
    SuggestCommand(String),
    /// `change_page` (client only).
    ChangePage(i32),
    /// An action type this client does not know (its id); it sends nothing.
    Other(String),
}

/// The dialog a `show_dialog` action opens.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DialogTarget {
    /// Sent inline.
    Inline(Box<Dialog>),
    /// A `minecraft:dialog` registry entry.
    Named(String),
}

/// What the client knows of the `minecraft:dialog` registry: names in id order, with the data
/// the server sent (none for entries of a pack the client knows).
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct DialogRegistry(pub(crate) Vec<(String, Option<Tag>)>);

impl DialogRegistry {
    /// The registry of a `registry_data` packet's entries.
    pub(crate) fn new(entries: impl Iterator<Item = (String, Option<AnonNbt>)>) -> Self {
        DialogRegistry(
            entries
                .map(|(k, v)| (k, v.and_then(|n| n.parse().ok().flatten())))
                .collect(),
        )
    }
}

/// A dialog event of the play state ([`TickEvents::dialogs`](crate::TickEvents::dialogs)).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DialogEvent {
    /// The server showed this dialog (it replaces any open one).
    Shown(Arc<Dialog>),
    /// The server closed the open dialog (`clear_dialog`).
    Cleared,
}

/// An input's value in a [`DialogAnswer`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum InputValue {
    /// A text box's text, as typed (vanilla's typing rules apply, see the module docs).
    Text(String),
    /// A checkbox.
    Bool(bool),
    /// A single-option input's option id.
    Option(String),
    /// A slider's value (snapped like vanilla's slider).
    Number(f32),
}

/// Which control an answer uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Button(usize),
    Exit,
    Close,
}

/// An answer to a dialog: a button (with the inputs' values) or Escape.
///
/// ```
/// use ghast_client::dialog::{DialogAnswer, InputValue};
///
/// // Type a password into the input `password` and press the first button.
/// let answer = DialogAnswer::button(0).with_value("password", InputValue::Text("hunter2".into()));
/// # let _ = answer;
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct DialogAnswer {
    pick: Pick,
    values: Vec<(String, InputValue)>,
    confirm_commands: bool,
}

impl DialogAnswer {
    /// Press button `index` of [`Dialog::buttons`].
    #[must_use]
    pub fn button(index: usize) -> Self {
        Self::new(Pick::Button(index))
    }

    /// Press the [`Dialog::exit`] button.
    #[must_use]
    pub fn exit() -> Self {
        Self::new(Pick::Exit)
    }

    /// Press Escape: runs [`Dialog::on_cancel`] and closes (refused when the dialog cannot be
    /// closed with Escape).
    #[must_use]
    pub fn close() -> Self {
        Self::new(Pick::Close)
    }

    fn new(pick: Pick) -> Self {
        DialogAnswer {
            pick,
            values: Vec::new(),
            confirm_commands: false,
        }
    }

    /// Sets an input's value (inputs left out keep their initial value).
    #[must_use]
    pub fn with_value(mut self, key: impl Into<String>, value: InputValue) -> Self {
        self.values.push((key.into(), value));
        self
    }

    /// Accept vanilla's command confirmation screen (default off): a command that vanilla would
    /// not send without asking (it does not parse with the server's command tree, needs
    /// permissions or has signable arguments) is sent as if the player accepted: an unsigned
    /// `chat_command` for the first two, the normal chat path (signed when there is a chat
    /// session) for a signable one. Off, such a command is not sent
    /// ([`DialogSent::CommandHeld`]) and the dialog stays open, as with a player who has not
    /// decided yet.
    #[must_use]
    pub fn confirm_commands(mut self, on: bool) -> Self {
        self.confirm_commands = on;
        self
    }

    /// Whether [`DialogAnswer::confirm_commands`] is on.
    pub(crate) fn confirms(&self) -> bool {
        self.confirm_commands
    }
}

/// Why an answer sent nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DialogRefused {
    /// No dialog is open.
    NoDialog,
    /// The dialog has no button with this index.
    NoButton(usize),
    /// The dialog has no exit button.
    NoExit,
    /// Escape does not close this dialog.
    CannotClose,
    /// No input has this key.
    UnknownInput(String),
    /// The value does not fit the input (wrong kind, unknown option id).
    BadValue(String),
}

impl std::fmt::Display for DialogRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DialogRefused::NoDialog => f.write_str("no dialog is open"),
            DialogRefused::NoButton(i) => write!(f, "the dialog has no button {i}"),
            DialogRefused::NoExit => f.write_str("the dialog has no exit button"),
            DialogRefused::CannotClose => f.write_str("the dialog cannot be closed with Escape"),
            DialogRefused::UnknownInput(k) => write!(f, "the dialog has no input {k:?}"),
            DialogRefused::BadValue(k) => write!(f, "the value does not fit input {k:?}"),
        }
    }
}

impl std::error::Error for DialogRefused {}

/// What vanilla's check of a dialog command found (`ClientPacketListener.verifyCommand`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CommandCheck {
    /// Parses, nothing to sign, no permissions needed: sent at once.
    NoIssues,
    /// Does not parse with the server's command tree (or there is none).
    ParseErrors,
    /// Has signable arguments.
    SignatureRequired,
    /// Parses only with the player's permissions.
    PermissionsRequired,
}

/// What an answer sent.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DialogSent {
    /// Nothing (no action, a client-only action, or another dialog opened).
    Nothing,
    /// The command (without `/`), as an unsigned `chat_command` (or through the chat path, see
    /// [`DialogAnswer::confirm_commands`]).
    Command(String),
    /// Vanilla would ask the player before sending this command: nothing was sent and the dialog
    /// stays open.
    CommandHeld {
        /// The command (without `/`).
        command: String,
        /// Why vanilla asks.
        check: CommandCheck,
    },
    /// The configuration state does not run commands (vanilla logs it and the dialog stays
    /// open).
    CommandNotSupported(String),
    /// `custom_click_action` with this id.
    Custom(String),
}

/// The result of an answer.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DialogOutcome {
    /// What was sent.
    pub sent: DialogSent,
    /// The dialog open afterwards (the same one, another one it opened, or none).
    pub open: Option<Arc<Dialog>>,
    /// A "waiting for response" screen is up (`after_action: wait_for_response`).
    pub waiting: bool,
}

/// What an answer does, before the state decides about commands.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Effect {
    /// Nothing is sent.
    Nothing,
    /// A command line (the `/` removed).
    Command(String),
    /// `custom_click_action` (payload already in Java's order).
    Custom { id: String, payload: Option<Tag> },
}

/// The screen after an answer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Next {
    /// The dialog stays.
    Stay,
    /// The dialog closes.
    Closed,
    /// The waiting screen.
    Waiting,
    /// Another dialog opens.
    Open(Arc<Dialog>),
}

/// One answer resolved against `dialog` (pure; vanilla's `DialogScreen.runAction`).
pub(crate) fn resolve(
    dialog: &Dialog,
    answer: &DialogAnswer,
) -> Result<(Effect, Next), DialogRefused> {
    let (action, after) = match answer.pick {
        Pick::Button(i) => {
            let b = dialog.buttons.get(i).ok_or(DialogRefused::NoButton(i))?;
            (b.action.as_ref(), dialog.after_action)
        }
        Pick::Exit => {
            let b = dialog.exit.as_ref().ok_or(DialogRefused::NoExit)?;
            (b.action.as_ref(), dialog.after_action)
        }
        Pick::Close => {
            if !dialog.can_close_with_escape {
                return Err(DialogRefused::CannotClose);
            }
            (dialog.on_cancel.as_ref(), AfterAction::Close)
        }
    };
    let values = values(dialog, &answer.values)?;
    let next = match after {
        AfterAction::Close => Next::Closed,
        AfterAction::None => Next::Stay,
        AfterAction::WaitForResponse => Next::Waiting,
    };
    let Some(action) = action else {
        return Ok((Effect::Nothing, next));
    };
    Ok(match action {
        DialogAction::RunCommand(c) => (Effect::Command(trim_slash(c).to_owned()), next),
        DialogAction::CommandTemplate(t) => {
            let line = instantiate(t, &values);
            (Effect::Command(trim_slash(&line).to_owned()), next)
        }
        DialogAction::Custom { id, payload } => (
            Effect::Custom {
                id: id.clone(),
                payload: payload
                    .as_ref()
                    .and_then(|p| p.parse().ok().flatten())
                    .map(java_order),
            },
            next,
        ),
        DialogAction::CustomTemplate { id, additions } => {
            let mut c = match additions.as_ref().and_then(|a| a.parse().ok().flatten()) {
                Some(Tag::Compound(c)) => java_order_compound(c),
                _ => Compound::default(),
            };
            for (k, v) in java_iteration(values.iter().map(|(k, v)| (k.clone(), v.tag.clone()))) {
                match c.0.iter_mut().find(|(n, _)| *n == k) {
                    Some(e) => e.1 = v,
                    None => c.0.push((k, v)),
                }
            }
            (
                Effect::Custom {
                    id: id.clone(),
                    payload: Some(Tag::Compound(java_order_compound(c))),
                },
                next,
            )
        }
        DialogAction::ShowDialog(target) => {
            // DialogScreen: the dialog opens over the screen the after-action leaves.
            let opened = match target {
                DialogTarget::Inline(d) => Some((**d).clone()),
                DialogTarget::Named(name) => dialog.registry.as_ref().and_then(|r| {
                    let i = r.0.iter().position(|(n, _)| n == name)?;
                    Some(from_registry(r, i))
                }),
            };
            match opened {
                Some(d) => (Effect::Nothing, Next::Open(Arc::new(d))),
                None => {
                    log::warn!("dialog: cannot open the unknown dialog {target:?}");
                    (Effect::Nothing, next)
                }
            }
        }
        DialogAction::OpenUrl(_)
        | DialogAction::CopyToClipboard(_)
        | DialogAction::SuggestCommand(_)
        | DialogAction::ChangePage(_)
        | DialogAction::Other(_) => (Effect::Nothing, next),
    })
}

/// `Commands.trimOptionalPrefix`.
fn trim_slash(s: &str) -> &str {
    s.strip_prefix('/').unwrap_or(s)
}

/// One input's value: its template text and its tag.
#[derive(Debug, Clone)]
struct Value {
    text: String,
    tag: Tag,
}

/// The value getters of the dialog's inputs (`DialogControlSet.valueGetters`), keyed in input
/// order (a later input with the same key replaces the earlier one in place, like `HashMap.put`).
fn values(
    dialog: &Dialog,
    given: &[(String, InputValue)],
) -> Result<Vec<(String, Value)>, DialogRefused> {
    for (k, _) in given {
        if !dialog.inputs.iter().any(|i| i.key == *k) {
            return Err(DialogRefused::UnknownInput(k.clone()));
        }
    }
    let mut out: Vec<(String, Value)> = Vec::new();
    for input in &dialog.inputs {
        let v = given
            .iter()
            .rev()
            .find(|(k, _)| *k == input.key)
            .map(|(_, v)| v);
        let bad = || DialogRefused::BadValue(input.key.clone());
        let value = match &input.kind {
            InputKind::Text {
                initial,
                max_length,
                multiline,
            } => {
                let text = match v {
                    None => truncate_utf16(initial, *max_length),
                    Some(InputValue::Text(t)) => typed(t, *max_length, *multiline),
                    Some(_) => return Err(bad()),
                };
                Value {
                    text: escape_without_quotes(&text),
                    tag: Tag::String(text),
                }
            }
            InputKind::Bool {
                initial,
                on_true,
                on_false,
            } => {
                let on = match v {
                    None => *initial,
                    Some(InputValue::Bool(b)) => *b,
                    Some(_) => return Err(bad()),
                };
                Value {
                    text: if on {
                        on_true.clone()
                    } else {
                        on_false.clone()
                    },
                    tag: Tag::Byte(i8::from(on)),
                }
            }
            InputKind::SingleOption { options, initial } => {
                let id = match v {
                    None => options.get(*initial).map(|o| o.0.clone()).ok_or_else(bad)?,
                    Some(InputValue::Option(id)) if options.iter().any(|o| o.0 == *id) => {
                        id.clone()
                    }
                    Some(_) => return Err(bad()),
                };
                Value {
                    text: id.clone(),
                    tag: Tag::String(id),
                }
            }
            InputKind::NumberRange {
                start,
                end,
                initial,
                step,
            } => {
                let r = Range {
                    start: *start,
                    end: *end,
                    initial: *initial,
                    step: *step,
                };
                let slider = match v {
                    None => r.initial_slider(),
                    Some(InputValue::Number(n)) => r.to_slider(*n).clamp(0.0, 1.0),
                    Some(_) => return Err(bad()),
                };
                let n = r.scaled(slider);
                Value {
                    text: number_text(n),
                    tag: Tag::Float(n),
                }
            }
            InputKind::Other(_) => continue,
        };
        match out.iter_mut().find(|(k, _)| *k == input.key) {
            Some(e) => e.1 = value,
            None => out.push((input.key.clone(), value)),
        }
    }
    Ok(out)
}

/// `EditBox` typing: disallowed characters dropped, cut at `max` UTF-16 units (not inside a
/// surrogate pair).
fn typed(s: &str, max: i32, multiline: bool) -> String {
    let filtered: String = s
        .chars()
        .filter(|&c| (c != '§' && c >= ' ' && c != '\u{7f}') || (multiline && c == '\n'))
        .collect();
    truncate_utf16(&filtered, max)
}

/// The first `max` UTF-16 units (a pair that does not fit is dropped whole).
fn truncate_utf16(s: &str, max: i32) -> String {
    let max = usize::try_from(max).unwrap_or(0);
    let mut n = 0;
    let mut out = String::new();
    for c in s.chars() {
        n += c.len_utf16();
        if n > max {
            break;
        }
        out.push(c);
    }
    out
}

/// `StringTag.escapeWithoutQuotes`.
fn escape_without_quotes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' | '\'' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if c < ' ' => out.push_str(&format!("\\x{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// `ParsedTemplate.instantiate`: `$(key)` replaced by the key's text ("" for unknown keys). A
/// template that does not parse (vanilla would not accept the dialog) is used as it is.
fn instantiate(template: &str, values: &[(String, Value)]) -> String {
    let b = template.as_bytes();
    let mut out = String::new();
    let mut start = 0;
    let mut i = 0;
    let mut any = false;
    while let Some(off) = template[i..].find('$') {
        let at = i + off;
        if at + 1 < b.len() && b[at + 1] == b'(' {
            let Some(close) = template[at + 1..].find(')').map(|c| at + 1 + c) else {
                return template.to_owned();
            };
            let name = &template[at + 2..close];
            if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return template.to_owned();
            }
            out.push_str(&template[start..at]);
            if let Some((_, v)) = values.iter().find(|(k, _)| k == name) {
                out.push_str(&v.text);
            }
            any = true;
            start = close + 1;
            i = start;
        } else {
            i = at + 1;
        }
    }
    if !any {
        return template.to_owned();
    }
    out.push_str(&template[start..]);
    out
}

/// A number range input's slider (`NumberRangeInput.RangeInfo`, float maths).
struct Range {
    start: f32,
    end: f32,
    initial: Option<f32>,
    step: Option<f32>,
}

impl Range {
    fn initial_value(&self) -> f32 {
        self.initial.unwrap_or((self.start + self.end) / 2.0)
    }

    fn to_slider(&self, v: f32) -> f32 {
        if self.start == self.end {
            0.5
        } else {
            (v - self.start) / (self.end - self.start)
        }
    }

    fn initial_slider(&self) -> f32 {
        self.to_slider(self.initial_value())
    }

    /// `computeScaledValue`.
    fn scaled(&self, slider: f32) -> f32 {
        let in_range = self.start + slider * (self.end - self.start);
        let Some(step) = self.step else {
            return in_range;
        };
        let initial = self.initial_value();
        let steps = java_round((in_range - initial) / step);
        let result = initial + steps as f32 * step;
        let s = self.to_slider(result);
        if (0.0..=1.0).contains(&s) {
            result
        } else {
            initial + (steps - steps.signum()) as f32 * step
        }
    }
}

/// Java's `Math.round(float)`.
fn java_round(x: f32) -> i32 {
    if x.is_nan() {
        0
    } else {
        (f64::from(x) + 0.5).floor() as i32
    }
}

/// `NumberRangeHandler.valueToString`: whole numbers as integers, else `Float.toString`.
fn number_text(v: f32) -> String {
    let i = v as i32;
    if i as f32 == v {
        i.to_string()
    } else {
        java_float_string(v)
    }
}

/// Java's `Float.toString` (shortest round-trip digits; plain notation for 1e-3 <= |v| < 1e7,
/// else `d.dddE±n`).
fn java_float_string(v: f32) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let sign = if v < 0.0 { "-" } else { "" };
    let a = v.abs();
    let e = format!("{a:e}");
    let (mant, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mant.chars().filter(char::is_ascii_digit).collect();
    if (1e-3..1e7).contains(&a) {
        let point = exp + 1;
        let s = if point <= 0 {
            format!("0.{}{digits}", "0".repeat((-point) as usize))
        } else if point as usize >= digits.len() {
            format!("{digits}{}.0", "0".repeat(point as usize - digits.len()))
        } else {
            format!(
                "{}.{}",
                &digits[..point as usize],
                &digits[point as usize..]
            )
        };
        format!("{sign}{s}")
    } else {
        let rest = if digits.len() > 1 { &digits[1..] } else { "0" };
        format!("{sign}{}.{rest}E{exp}", &digits[..1])
    }
}

/// Java's `String.hashCode`.
fn java_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

/// The iteration order of a Java `HashMap` filled with `entries` in this order (equal keys are
/// assumed merged already): by bucket of the final table, equal buckets in insertion order
/// (resizes keep the relative order; bins long enough to become trees are not modelled).
fn java_iteration<V>(entries: impl Iterator<Item = (String, V)>) -> Vec<(String, V)> {
    let mut v: Vec<(String, V)> = entries.collect();
    let mut cap = 16usize;
    while v.len() > cap / 4 * 3 {
        cap *= 2;
    }
    v.sort_by_key(|(k, _)| {
        let h = java_hash(k) as u32;
        (h ^ (h >> 16)) as usize & (cap - 1)
    });
    v
}

/// A compound (and the compounds in it) in the order vanilla's `CompoundTag` writes it.
fn java_order_compound(c: Compound) -> Compound {
    Compound(java_iteration(
        c.0.into_iter().map(|(k, t)| (k, java_order(t))),
    ))
}

/// A tag with its compounds in vanilla's write order.
fn java_order(t: Tag) -> Tag {
    match t {
        Tag::Compound(c) => Tag::Compound(java_order_compound(c)),
        Tag::List(l) => Tag::List(List {
            elem_type: l.elem_type,
            items: l.items.into_iter().map(java_order).collect(),
        }),
        t => t,
    }
}

/// The `custom_click_action` body: the id, then the payload tag length-prefixed.
pub(crate) fn custom_click_body(
    id: &str,
    payload: Option<&AnonNbt>,
) -> Result<bytes::Bytes, ghast_protocol::EncodeError> {
    use ghast_protocol::McWrite;
    let mut b = bytes::BytesMut::new();
    b.put_string(id)?;
    let nbt = payload.map_or(&[0u8][..], |p| &p.as_bytes()[..]);
    b.put_len_varint(nbt.len())?;
    b.extend_from_slice(nbt);
    Ok(b.freeze())
}

/// An effect's payload as wire NBT.
pub(crate) fn payload_nbt(payload: Option<&Tag>) -> Option<AnonNbt> {
    payload.and_then(|t| AnonNbt::from_tag(t).ok())
}

// ---- decoding ----

/// A dialog sent inline.
pub(crate) fn decode(nbt: &AnonNbt, registry: Option<&Arc<DialogRegistry>>) -> Dialog {
    let tag = nbt.parse().ok().flatten();
    let mut d = match &tag {
        Some(Tag::Compound(c)) => from_compound(c, registry),
        _ => unresolved(None, registry),
    };
    d.nbt = Some(nbt.clone());
    d
}

/// Registry entry `id` (play `show_dialog` by reference).
pub(crate) fn decode_reference(id: i32, registry: Option<&Arc<DialogRegistry>>) -> Dialog {
    match (registry, usize::try_from(id)) {
        (Some(r), Ok(i)) if i < r.0.len() => from_registry(r, i),
        _ => unresolved(Some(format!("#{id}")), registry),
    }
}

fn from_registry(r: &Arc<DialogRegistry>, i: usize) -> Dialog {
    let (name, data) = &r.0[i];
    let mut d = match data {
        Some(Tag::Compound(c)) => {
            let mut d = from_compound(c, Some(r));
            d.nbt = data.as_ref().and_then(|t| AnonNbt::from_tag(t).ok());
            d
        }
        _ => unresolved(None, Some(r)),
    };
    d.registry_name = Some(name.clone());
    d
}

fn unresolved(name: Option<String>, registry: Option<&Arc<DialogRegistry>>) -> Dialog {
    Dialog {
        kind: DialogKind::Unresolved,
        title: String::new(),
        external_title: None,
        can_close_with_escape: true,
        pause: true,
        after_action: AfterAction::Close,
        body: Vec::new(),
        inputs: Vec::new(),
        buttons: Vec::new(),
        exit: None,
        on_cancel: None,
        registry_name: name,
        nbt: None,
        registry: registry.cloned(),
    }
}

/// A list element: vanilla writes the elements of a mixed list as compounds holding the
/// value under the empty key.
fn element(t: &Tag) -> &Tag {
    match t {
        Tag::Compound(c) if c.0.len() == 1 && c.0[0].0.is_empty() => &c.0[0].1,
        t => t,
    }
}

/// An id without the default namespace.
fn short(id: &str) -> &str {
    id.strip_prefix("minecraft:").unwrap_or(id)
}

fn text(t: Option<&Tag>) -> String {
    match t {
        Some(Tag::String(s)) => s.clone(),
        Some(t) => AnonNbt::from_tag(t)
            .map(|n| ghast_protocol::chat::nbt_to_plain_text(&n))
            .unwrap_or_default(),
        None => String::new(),
    }
}

fn string(c: &Compound, k: &str) -> Option<String> {
    match c.get(k) {
        Some(Tag::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn number(c: &Compound, k: &str) -> Option<f64> {
    Some(match c.get(k)? {
        Tag::Byte(v) => f64::from(*v),
        Tag::Short(v) => f64::from(*v),
        Tag::Int(v) => f64::from(*v),
        Tag::Long(v) => *v as f64,
        Tag::Float(v) => f64::from(*v),
        Tag::Double(v) => *v,
        _ => return None,
    })
}

fn boolean(c: &Compound, k: &str, default: bool) -> bool {
    match c.get(k) {
        Some(Tag::String(s)) => s == "true",
        Some(_) => number(c, k).is_some_and(|v| v != 0.0),
        None => default,
    }
}

fn int(c: &Compound, k: &str, default: i32) -> i32 {
    number(c, k).map_or(default, |v| v as i32)
}

fn float(c: &Compound, k: &str) -> Option<f32> {
    number(c, k).map(|v| v as f32)
}

fn nbt_of(t: Option<&Tag>) -> Option<AnonNbt> {
    t.and_then(|t| AnonNbt::from_tag(t).ok())
}

fn from_compound(c: &Compound, registry: Option<&Arc<DialogRegistry>>) -> Dialog {
    let ty = string(c, "type").unwrap_or_default();
    let button = |t: Option<&Tag>| match t {
        Some(Tag::Compound(b)) => Some(button_of(b, registry)),
        _ => None,
    };
    let exit = button(c.get("exit_action"));
    let columns = int(c, "columns", 2);
    let (kind, buttons, on_cancel) = match short(&ty) {
        "notice" => {
            let b = button(c.get("action")).unwrap_or_else(|| DialogButton {
                label: "gui.ok".into(),
                tooltip: None,
                width: 150,
                action: None,
            });
            let cancel = b.action.clone();
            (DialogKind::Notice, vec![b], cancel)
        }
        "confirmation" => {
            let yes = button(c.get("yes"));
            let no = button(c.get("no"));
            let cancel = no.as_ref().and_then(|b| b.action.clone());
            (
                DialogKind::Confirmation,
                yes.into_iter().chain(no).collect(),
                cancel,
            )
        }
        "multi_action" => {
            let actions = match c.get("actions") {
                Some(Tag::List(l)) => l
                    .items
                    .iter()
                    .filter_map(|t| button(Some(element(t))))
                    .collect(),
                _ => Vec::new(),
            };
            let cancel = exit.as_ref().and_then(|b| b.action.clone());
            (DialogKind::MultiAction { columns }, actions, cancel)
        }
        "dialog_list" => {
            let width = int(c, "button_width", 150);
            let buttons = list_targets(c.get("dialogs"), registry)
                .into_iter()
                .map(|t| {
                    let label = match &t {
                        DialogTarget::Inline(d) => {
                            d.external_title.clone().unwrap_or_else(|| d.title.clone())
                        }
                        // Only the entry's titles (a full decode could loop through lists
                        // that list each other).
                        DialogTarget::Named(n) => registry
                            .and_then(|r| r.0.iter().find(|(k, _)| k == n))
                            .and_then(|(_, t)| t.as_ref()?.as_compound())
                            .map_or_else(
                                || n.clone(),
                                |d| text(d.get("external_title").or_else(|| d.get("title"))),
                            ),
                    };
                    DialogButton {
                        label,
                        tooltip: None,
                        width,
                        action: Some(DialogAction::ShowDialog(t)),
                    }
                })
                .collect();
            let cancel = exit.as_ref().and_then(|b| b.action.clone());
            (DialogKind::DialogList { columns }, buttons, cancel)
        }
        "server_links" => {
            let cancel = exit.as_ref().and_then(|b| b.action.clone());
            (DialogKind::ServerLinks { columns }, Vec::new(), cancel)
        }
        other => (DialogKind::Other(other.to_owned()), Vec::new(), None),
    };
    let after_action = match string(c, "after_action").as_deref() {
        Some("none") => AfterAction::None,
        Some("wait_for_response") => AfterAction::WaitForResponse,
        _ => AfterAction::Close,
    };
    let body = match c.get("body") {
        Some(Tag::List(l)) => l.items.iter().map(|t| body_of(element(t))).collect(),
        Some(t) => vec![body_of(t)],
        None => Vec::new(),
    };
    let inputs = match c.get("inputs") {
        Some(Tag::List(l)) => l
            .items
            .iter()
            .filter_map(|t| element(t).as_compound().map(input_of))
            .collect(),
        _ => Vec::new(),
    };
    Dialog {
        kind,
        title: text(c.get("title")),
        external_title: c.get("external_title").map(|t| text(Some(t))),
        can_close_with_escape: boolean(c, "can_close_with_escape", true),
        pause: boolean(c, "pause", true),
        after_action,
        body,
        inputs,
        buttons,
        exit: if matches!(short(&ty), "notice" | "confirmation") {
            None
        } else {
            exit
        },
        on_cancel,
        registry_name: None,
        nbt: None,
        registry: registry.cloned(),
    }
}

/// A `dialog_list`'s `dialogs` (a holder set: an id, an inline dialog, or a list of them; tags
/// are skipped).
fn list_targets(t: Option<&Tag>, registry: Option<&Arc<DialogRegistry>>) -> Vec<DialogTarget> {
    let one = |t: &Tag| match t {
        Tag::String(s) if !s.starts_with('#') => Some(DialogTarget::Named(qualified(s))),
        Tag::Compound(c) => Some(DialogTarget::Inline(Box::new(from_compound(c, registry)))),
        _ => None,
    };
    match t {
        Some(Tag::List(l)) => l.items.iter().filter_map(|t| one(element(t))).collect(),
        Some(t) => one(t).into_iter().collect(),
        None => Vec::new(),
    }
}

/// An id with the default namespace.
fn qualified(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

fn button_of(b: &Compound, registry: Option<&Arc<DialogRegistry>>) -> DialogButton {
    DialogButton {
        label: text(b.get("label")),
        tooltip: b.get("tooltip").map(|t| text(Some(t))),
        width: int(b, "width", 150),
        action: match b.get("action") {
            Some(Tag::Compound(a)) => Some(action_of(a, registry)),
            _ => None,
        },
    }
}

fn action_of(a: &Compound, registry: Option<&Arc<DialogRegistry>>) -> DialogAction {
    let ty = string(a, "type").unwrap_or_default();
    let s = |k| string(a, k).unwrap_or_default();
    match short(&ty) {
        "run_command" => DialogAction::RunCommand(s("command")),
        "dynamic/run_command" => DialogAction::CommandTemplate(s("template")),
        "custom" => DialogAction::Custom {
            id: qualified(&s("id")),
            payload: nbt_of(a.get("payload")),
        },
        "dynamic/custom" => DialogAction::CustomTemplate {
            id: qualified(&s("id")),
            additions: nbt_of(a.get("additions")),
        },
        "show_dialog" => match a.get("dialog") {
            Some(Tag::String(n)) => DialogAction::ShowDialog(DialogTarget::Named(qualified(n))),
            Some(Tag::Compound(c)) => {
                let mut d = from_compound(c, registry);
                d.nbt = nbt_of(a.get("dialog"));
                DialogAction::ShowDialog(DialogTarget::Inline(Box::new(d)))
            }
            _ => DialogAction::Other(ty.clone()),
        },
        "open_url" => DialogAction::OpenUrl(s("url")),
        "copy_to_clipboard" => DialogAction::CopyToClipboard(s("value")),
        "suggest_command" => DialogAction::SuggestCommand(s("command")),
        "change_page" => DialogAction::ChangePage(int(a, "page", 1)),
        _ => DialogAction::Other(ty.clone()),
    }
}

fn body_of(t: &Tag) -> DialogBody {
    let Some(c) = t.as_compound() else {
        return DialogBody::Other(String::new());
    };
    let ty = string(c, "type").unwrap_or_default();
    match short(&ty) {
        "plain_message" => DialogBody::Text(text(c.get("contents"))),
        "item" => {
            let (id, count) = match c.get("item") {
                Some(Tag::Compound(i)) => (string(i, "id").unwrap_or_default(), int(i, "count", 1)),
                Some(Tag::String(s)) => (s.clone(), 1),
                _ => (String::new(), 1),
            };
            let description = match c.get("description") {
                Some(Tag::Compound(d)) if d.get("contents").is_some() => {
                    Some(text(d.get("contents")))
                }
                Some(t) => Some(text(Some(t))),
                None => None,
            };
            DialogBody::Item {
                id,
                count,
                description,
            }
        }
        other => DialogBody::Other(other.to_owned()),
    }
}

fn input_of(c: &Compound) -> DialogInput {
    let ty = string(c, "type").unwrap_or_default();
    let kind = match short(&ty) {
        "text" => InputKind::Text {
            initial: string(c, "initial").unwrap_or_default(),
            max_length: int(c, "max_length", 32),
            multiline: c.get("multiline").is_some(),
        },
        "boolean" => InputKind::Bool {
            initial: boolean(c, "initial", false),
            on_true: string(c, "on_true").unwrap_or_else(|| "true".into()),
            on_false: string(c, "on_false").unwrap_or_else(|| "false".into()),
        },
        "single_option" => {
            let options: Vec<(String, String, bool)> = match c.get("options") {
                Some(Tag::List(l)) => l
                    .items
                    .iter()
                    .filter_map(|t| match element(t) {
                        Tag::String(id) => Some((id.clone(), id.clone(), false)),
                        Tag::Compound(o) => {
                            let id = string(o, "id")?;
                            let display = o
                                .get("display")
                                .map_or_else(|| id.clone(), |t| text(Some(t)));
                            Some((id, display, boolean(o, "initial", false)))
                        }
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            // The initial entry, else the first.
            let initial = options.iter().position(|o| o.2).unwrap_or(0);
            InputKind::SingleOption {
                options: options.into_iter().map(|(id, d, _)| (id, d)).collect(),
                initial,
            }
        }
        "number_range" => InputKind::NumberRange {
            start: float(c, "start").unwrap_or(0.0),
            end: float(c, "end").unwrap_or(0.0),
            initial: float(c, "initial"),
            step: float(c, "step"),
        },
        other => InputKind::Other(other.to_owned()),
    };
    DialogInput {
        key: string(c, "key").unwrap_or_default(),
        label: text(c.get("label")),
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn nbt(s: &str) -> AnonNbt {
        AnonNbt::from_bytes(bytes::Bytes::from(hex(s))).unwrap()
    }

    /// The vanilla client's answers (`tests/golden/26.2/dialog_answers.txt`, written by the
    /// vanilla 26.2 classes): decode the dialog the server sent, answer it with the same inputs,
    /// and the packet body must be byte-identical (payload compounds in Java's `HashMap` order,
    /// SNBT escaping, the slider's snapping and number text, the length-prefixed tag).
    #[test]
    fn answers_match_the_vanilla_client() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/golden/26.2/dialog_answers.txt"
        );
        let text = std::fs::read_to_string(path).unwrap();
        let mut n = 0;
        for line in text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let f: Vec<&str> = line.split(' ').collect();
            let (name, dialog, pick, given) = (f[0], nbt(f[1]), f[2], nbt(f[3]));
            let d = decode(&dialog, None);
            let mut answer = match pick {
                "exit" => DialogAnswer::exit(),
                "close" => DialogAnswer::close(),
                p => DialogAnswer::button(p.strip_prefix("button_").unwrap().parse().unwrap()),
            };
            let Some(Tag::Compound(given)) = given.parse().unwrap() else {
                panic!("{name}: values")
            };
            for (k, v) in given.0 {
                let kind = &d.inputs.iter().find(|i| i.key == k).unwrap().kind;
                let v = match (kind, v) {
                    (InputKind::Text { .. }, Tag::String(s)) => InputValue::Text(s),
                    (InputKind::SingleOption { .. }, Tag::String(s)) => InputValue::Option(s),
                    (InputKind::Bool { .. }, Tag::Byte(b)) => InputValue::Bool(b != 0),
                    (InputKind::NumberRange { .. }, Tag::Float(x)) => InputValue::Number(x),
                    (k, v) => panic!("{name}: {k:?} {v:?}"),
                };
                answer = answer.with_value(k, v);
            }
            let (effect, _) = resolve(&d, &answer).unwrap();
            let got = match effect {
                Effect::Nothing => None,
                Effect::Custom { id, payload } => Some((
                    "custom_click_action",
                    custom_click_body(&id, payload_nbt(payload.as_ref()).as_ref()).unwrap(),
                )),
                Effect::Command(c) => {
                    use ghast_protocol::McWrite;
                    let mut b = bytes::BytesMut::new();
                    b.put_string(&c).unwrap();
                    Some(("chat_command", b.freeze()))
                }
            };
            match (f[4], got) {
                ("none" | "client", None) => {}
                (kind, Some((k, body))) => {
                    assert_eq!(kind, k, "{name}");
                    assert_eq!(hex(f[5]), body.to_vec(), "{name}");
                }
                (kind, got) => panic!("{name}: vanilla {kind}, ours {got:?}"),
            }
            n += 1;
        }
        assert!(n >= 12, "{n}");
    }

    /// The decoded model of a dialog.
    #[test]
    fn decodes_the_dialog_model() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/golden/26.2/dialog_answers.txt"
        ))
        .unwrap();
        let line = text
            .lines()
            .find(|l| l.starts_with("template_login "))
            .unwrap();
        let d = decode(&nbt(line.split(' ').nth(1).unwrap()), None);
        assert_eq!(d.kind, DialogKind::MultiAction { columns: 2 });
        assert_eq!(d.title, "Login");
        assert!(d.can_close_with_escape && d.pause);
        assert_eq!(d.after_action, AfterAction::Close);
        assert_eq!(d.buttons.len(), 6);
        assert_eq!(d.buttons[0].label, "Run");
        assert_eq!(d.buttons[0].width, 150);
        assert_eq!(
            d.buttons[0].action,
            Some(DialogAction::RunCommand("/login fixed".into()))
        );
        assert_eq!(d.buttons[3].action, None);
        assert_eq!(
            d.buttons[5].action,
            Some(DialogAction::OpenUrl("https://example.com".into()))
        );
        assert_eq!(d.exit.as_ref().unwrap().label, "Exit");
        assert!(matches!(
            d.on_cancel,
            Some(DialogAction::CustomTemplate { .. })
        ));
        let keys: Vec<&str> = d.inputs.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["pw", "n", "big", "keep"]);
        assert_eq!(
            d.inputs[3].kind,
            InputKind::Bool {
                initial: true,
                on_true: "yes".into(),
                on_false: "no".into()
            }
        );
        // A notice without an action has vanilla's OK button.
        let mut c = Compound::default();
        c.0.push(("type".into(), Tag::String("minecraft:notice".into())));
        c.0.push(("title".into(), Tag::String("Hi".into())));
        let d = decode(&AnonNbt::from_tag(&Tag::Compound(c)).unwrap(), None);
        assert_eq!(d.buttons.len(), 1);
        assert_eq!(d.buttons[0].label, "gui.ok");
        assert_eq!(d.on_cancel, None);
        // Escape on a dialog that forbids it.
        let mut d2 = d.clone();
        d2.can_close_with_escape = false;
        assert_eq!(
            resolve(&d2, &DialogAnswer::close()),
            Err(DialogRefused::CannotClose)
        );
        assert_eq!(
            resolve(&d, &DialogAnswer::button(1)),
            Err(DialogRefused::NoButton(1))
        );
        assert_eq!(
            resolve(
                &d,
                &DialogAnswer::button(0).with_value("x", InputValue::Bool(true))
            ),
            Err(DialogRefused::UnknownInput("x".into()))
        );
    }

    /// Registry dialogs: by id, with and without data, and `show_dialog` by name.
    #[test]
    fn registry_dialogs() {
        let notice = |title: &str, action: Option<Tag>| {
            let mut c = Compound::default();
            c.0.push(("type".into(), Tag::String("notice".into())));
            c.0.push(("title".into(), Tag::String(title.into())));
            if let Some(a) = action {
                let mut b = Compound::default();
                b.0.push(("label".into(), Tag::String("Go".into())));
                b.0.push(("action".into(), a));
                c.0.push(("action".into(), Tag::Compound(b)));
            }
            Tag::Compound(c)
        };
        let mut show = Compound::default();
        show.0
            .push(("type".into(), Tag::String("show_dialog".into())));
        show.0
            .push(("dialog".into(), Tag::String("test:second".into())));
        let reg = Arc::new(DialogRegistry(vec![
            ("minecraft:server_links".into(), None),
            (
                "test:first".into(),
                Some(notice("First", Some(Tag::Compound(show)))),
            ),
            ("test:second".into(), Some(notice("Second", None))),
        ]));
        let d = decode_reference(0, Some(&reg));
        assert_eq!(d.kind, DialogKind::Unresolved);
        assert_eq!(d.registry_name.as_deref(), Some("minecraft:server_links"));
        let d = decode_reference(1, Some(&reg));
        assert_eq!(
            (d.kind.clone(), d.title.as_str()),
            (DialogKind::Notice, "First")
        );
        let (effect, next) = resolve(&d, &DialogAnswer::button(0)).unwrap();
        assert_eq!(effect, Effect::Nothing);
        let Next::Open(second) = next else {
            panic!("{next:?}")
        };
        assert_eq!(second.title, "Second");
        assert_eq!(second.registry_name.as_deref(), Some("test:second"));
        assert_eq!(decode_reference(7, Some(&reg)).kind, DialogKind::Unresolved);
    }

    #[test]
    fn java_helpers() {
        // Float.toString / valueToString.
        for (v, s) in [
            (1.0f32, "1"),
            (2.5, "2.5"),
            (0.0004, "4.0E-4"),
            (0.001, "0.001"),
            (1.5e7, "15000000"),
            (1.5e10, "15000000000"),
            (-0.25, "-0.25"),
            (1234567.5, "1234567.5"),
            (1.25e-5, "1.25E-5"),
            (3.4e38, "3.4E38"),
        ] {
            let got = if v.abs() > 2.2e9 {
                java_float_string(v)
            } else {
                number_text(v)
            };
            if s == "15000000000" {
                assert_eq!(java_float_string(v), "1.5E10");
                continue;
            }
            assert_eq!(got, s, "{v}");
        }
        assert_eq!(java_round(2.5), 3);
        assert_eq!(java_round(-2.5), -2);
        assert_eq!(java_round(0.499_999_97), 0);
        assert_eq!(java_hash("Aa"), java_hash("BB"));
        assert_eq!(java_hash("hello"), 99_162_322);
        assert_eq!(
            escape_without_quotes("a\"b'c\\d\n\u{1}"),
            "a\\\"b\\'c\\\\d\\n\\x01"
        );
        let v = |k: &str, t: &str| {
            (
                k.to_owned(),
                Value {
                    text: t.into(),
                    tag: Tag::Byte(0),
                },
            )
        };
        let vals = [v("a", "1"), v("b", "2")];
        assert_eq!(instantiate("x $(a) $(b)$(c) y", &vals), "x 1 2 y");
        assert_eq!(instantiate("no vars", &vals), "no vars");
        assert_eq!(instantiate("$(a", &vals), "$(a");
        assert_eq!(typed("a§b\nc\u{7f}d", 10, false), "abcd");
        assert_eq!(typed("a\nb", 10, true), "a\nb");
        assert_eq!(typed("ab😀c", 3, false), "ab");
    }
}
