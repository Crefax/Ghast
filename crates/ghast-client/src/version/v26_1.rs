//! Protocol 775 (26.1 – 26.1.2): login with the profile id, the configuration state, and the
//! play packets of the modern client (`player_input`, `tick_end`, `player_loaded`, sequenced
//! block actions, unsigned chat with message acknowledgements).

use std::collections::HashMap;

use bytes::{Bytes, BytesMut};
use ghast_protocol::chat::nbt_to_plain_text;
use ghast_protocol::nbt::AnonNbt;
use ghast_protocol::types::{ByteString, LpVec3};
use ghast_protocol::v26_1::{configuration, handshaking, login, play};
use ghast_protocol::{DecodeError, EncodeError, FrameEncoder, McWrite, PacketFilter, RawPacket};

use super::{ClientVersion, GameVersion, V26_1, teleport_target, truncate_utf16};
use crate::common::{
    Abilities, AttributeUpdate, ChatMessage, ChatPosition, ConfigInbound, ConfigOutbound,
    EntityAction, GameMode, Health, Inbound, JoinGame, KnownPack, LoginInbound, MovePacket,
    Outbound, Pose, Teleport, Vec3,
};
use crate::interest::Interests;

use play::clientbound as cb;
use play::serverbound as sb;

/// Messages kept for acknowledgement (`LastSeenMessagesTracker(20)`).
const LAST_SEEN: usize = 20;
/// The client acknowledges once this many messages are pending (`markMessageAsProcessed`).
const ACK_AFTER: i32 = 64;

/// Per-connection wire state of protocol 775 (opaque).
#[derive(Debug, Default)]
pub struct Wire26_1 {
    /// `BlockStatePredictionHandler.currentSequenceNr` of the current level.
    sequence: i32,
    /// The look of the last movement packet (`use_item` carries the player's rotation).
    last_look: (f32, f32),
    /// The rotation the current batch's movement packet sends, if it sends one: the player's
    /// rotation at key-handling time (vanilla turns the mouse before the tick), which `use_item`
    /// carries; Grim checks it against the tick's rotation.
    tick_look: Option<(f32, f32)>,
    /// The current dimension (a respawn into another one starts a new level).
    dimension: String,
    /// Received signed messages not yet acknowledged.
    seen: LastSeen,
    /// Next expected `player_chat` global index.
    next_chat_index: i32,
    /// `minecraft:chat_type` registry keys in order.
    chat_types: Vec<String>,
    /// Cookies the server stored.
    cookies: HashMap<ByteString, Bytes>,
    /// Salt generator state.
    salt: u64,
    /// Registry names the inventory needs for hashed click stacks.
    hash_registries: ghast_inventory::v26_1::HashRegistries,
    /// This configuration's registry and tag bodies for the inventory (handed over at its end).
    inv_config: Vec<(bool, Bytes)>,
    /// The last `declare_commands` packet as one compressed frame (a vanilla tree is ~35 KB
    /// raw, a few KB compressed); inflated and parsed only when a command is sent.
    commands_raw: Option<Box<[u8]>>,
    /// The tree parsed from `commands_raw`.
    commands: Option<super::commands::CommandTree>,
    /// The chat session (online mode with profile keys): signs chat and command arguments.
    chat: Option<ChatSession>,
    /// Our entity id (entity events 24–28 set the permission level).
    entity_id: i32,
    /// The player's permission level (`LocalPlayer.permissions`; selectors need 2).
    op_level: u8,
}

/// `LocalChatSession` + its message encoder.
struct ChatSession {
    keys: std::sync::Arc<ghast_auth::chat::ProfileKeys>,
    chain: ghast_auth::chat::SignedMessageChain,
}

impl std::fmt::Debug for ChatSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatSession")
            .field("session", &self.chain.session_id())
            .finish_non_exhaustive()
    }
}

/// `UUID.randomUUID`.
fn random_uuid() -> uuid::Uuid {
    let mut b = [0u8; 16];
    ghast_auth::crypto::random_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(b)
}

/// `LastSeenMessagesTracker`: a ring of the last 20 signed messages seen.
#[derive(Debug, Default)]
struct LastSeen {
    entries: [Option<Bytes>; LAST_SEEN],
    tail: usize,
    offset: i32,
    last: Option<Bytes>,
    /// An acknowledgement is due (`offset > 64` after a new message).
    ack_due: bool,
}

impl LastSeen {
    /// `addPending(signature, wasShown)`: a message not shown (vanilla: blocked sender,
    /// failed validation, hidden chat) takes a slot without a signature.
    fn add(&mut self, sig: Bytes, shown: bool) {
        if self.last.as_ref() == Some(&sig) {
            return;
        }
        self.last = Some(sig.clone());
        self.entries[self.tail] = shown.then_some(sig);
        self.tail = (self.tail + 1) % LAST_SEEN;
        self.offset += 1;
        if self.offset > ACK_AFTER {
            self.ack_due = true;
        }
    }

    /// `generateAndApplyUpdate`: offset, the acknowledged bit set (20 bits, 3 bytes) and the
    /// checksum of the acknowledged signatures.
    #[cfg(test)]
    fn update(&mut self) -> (i32, [u8; 3], u8) {
        self.update_with(|_| {})
    }

    /// [`LastSeen::update`], passing every acknowledged signature in order to `seen` (they
    /// are the signed body's last-seen list).
    fn update_with(&mut self, mut seen: impl FnMut(&Bytes)) -> (i32, [u8; 3], u8) {
        let offset = std::mem::take(&mut self.offset);
        self.ack_due = false;
        let mut bits = [0u8; 3];
        let mut checksum: i32 = 1;
        for i in 0..LAST_SEEN {
            if let Some(sig) = &self.entries[(self.tail + i) % LAST_SEEN] {
                bits[i / 8] |= 1 << (i % 8);
                checksum = checksum.wrapping_mul(31).wrapping_add(java_array_hash(sig));
                seen(sig);
            }
        }
        let b = checksum as u8;
        (offset, bits, if b == 0 { 1 } else { b })
    }
}

/// `Arrays.hashCode(byte[])`.
fn java_array_hash(b: &[u8]) -> i32 {
    b.iter().fold(1i32, |h, &x| {
        h.wrapping_mul(31).wrapping_add(i32::from(x as i8))
    })
}

impl Wire26_1 {
    /// Test hook: the look of the last movement packet.
    #[cfg(test)]
    pub(crate) fn set_last_look(&mut self, yaw: f32, pitch: f32) {
        self.last_look = (yaw, pitch);
    }

    /// The next prediction sequence number (`startPredicting`).
    fn next_sequence(&mut self) -> i32 {
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }

    fn next_salt(&mut self) -> i64 {
        if self.salt == 0 {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as u64);
            self.salt = t ^ (std::ptr::from_ref(self) as u64).rotate_left(32) | 1;
        }
        // xorshift64*
        let mut x = self.salt;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.salt = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D) as i64
    }

    /// Signs `content` with the chat session, if any (`signedMessageEncoder.pack`).
    fn sign(&mut self, content: &str, timestamp: i64, salt: i64, seen: &[Bytes]) -> Option<Bytes> {
        let s = self.chat.as_mut()?;
        match s.chain.sign(
            &s.keys,
            content,
            timestamp,
            salt,
            seen.iter().map(|b| &b[..]),
        ) {
            Ok(sig) => sig.map(Bytes::from),
            Err(e) => {
                log::warn!("chat signing failed: {e}");
                None
            }
        }
    }

    /// `sendChat`: a chat message (signed when there is a chat session).
    fn write_chat(&mut self, enc: &mut FrameEncoder, text: &str) -> Result<(), EncodeError> {
        let salt = self.next_salt();
        self.write_chat_at(enc, text, now_millis(), salt)
    }

    fn write_chat_at(
        &mut self,
        enc: &mut FrameEncoder,
        text: &str,
        timestamp: i64,
        salt: i64,
    ) -> Result<(), EncodeError> {
        let message = truncate_utf16(text, V26_1::CHAT_LIMIT);
        let mut seen = Vec::new();
        let signing = self.chat.is_some();
        let (offset, bits, checksum) = self.seen.update_with(|s| {
            if signing {
                seen.push(s.clone());
            }
        });
        let signature = self.sign(message, timestamp, salt, &seen);
        enc.write_packet(&sb::ChatMessage {
            message: ByteString::from(message),
            timestamp,
            salt,
            signature,
            offset,
            acknowledged: Bytes::copy_from_slice(&bits),
            checksum,
        })
    }

    /// `sendCommand`: `chat_command_signed` when the server's command tree gives the line
    /// signable (`minecraft:message`) arguments, else `chat_command`.
    fn write_command(&mut self, enc: &mut FrameEncoder, command: &str) -> Result<(), EncodeError> {
        let salt = self.next_salt();
        self.write_command_at(enc, command, now_millis(), salt)
    }

    fn write_command_at(
        &mut self,
        enc: &mut FrameEncoder,
        command: &str,
        timestamp: i64,
        salt: i64,
    ) -> Result<(), EncodeError> {
        if self.commands.is_none()
            && let Some(raw) = self.commands_raw.as_deref().and_then(inflate)
        {
            match raw.decode_as::<cb::DeclareCommands>() {
                Ok(p) => self.commands = super::commands::CommandTree::from_packet(&p),
                Err(e) => log::warn!("bad declare_commands packet: {e}"),
            }
            if self.commands.is_some() {
                self.commands_raw = None;
            }
        }
        let signable = self
            .commands
            .as_ref()
            .map(|t| t.signable_arguments(command, self.op_level >= 2))
            .unwrap_or_default();
        if signable.is_empty() {
            return enc.write_packet(&sb::ChatCommand {
                command: ByteString::from(command),
            });
        }
        let mut seen = Vec::new();
        let signing = self.chat.is_some();
        let (offset, bits, checksum) = self.seen.update_with(|s| {
            if signing {
                seen.push(s.clone());
            }
        });
        let mut argument_signatures = Vec::new();
        for (name, value) in signable {
            if let Some(signature) = self.sign(value, timestamp, salt, &seen) {
                argument_signatures.push(sb::ChatCommandSignedArgumentSignatures {
                    argument_name: ByteString::from(&*name),
                    signature,
                });
            }
        }
        enc.write_packet(&sb::ChatCommandSigned {
            command: ByteString::from(command),
            timestamp,
            salt,
            argument_signatures,
            message_count: offset,
            acknowledged: Bytes::copy_from_slice(&bits),
            checksum,
        })
    }

    fn new_level(&mut self, dimension: &str) {
        self.sequence = 0;
        dimension.clone_into(&mut self.dimension);
    }
}

/// `Instant.now()` in milliseconds.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Plain text of a network chat component.
fn plain(nbt: &AnonNbt) -> String {
    nbt_to_plain_text(nbt)
}

/// A disconnect reason as the JSON text component the session end reports.
fn reason_json(nbt: &AnonNbt) -> ByteString {
    let text = plain(nbt);
    let mut s = String::with_capacity(text.len() + 12);
    s.push_str("{\"text\":\"");
    for c in text.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            c if (c as u32) < 0x20 => s.push_str(&format!("\\u{:04x}", c as u32)),
            c => s.push(c),
        }
    }
    s.push_str("\"}");
    ByteString::from(s)
}

/// The vanilla en_us pattern of a chat type, with `%1` the sender and `%2` the content (and
/// `%3` the target where the type has one).
fn chat_pattern(chat_type: &str) -> &'static str {
    match chat_type.strip_prefix("minecraft:").unwrap_or(chat_type) {
        "say_command" => "[%1] %2",
        "emote_command" => "* %1 %2",
        "msg_command_incoming" => "%1 whispers to you: %2",
        "msg_command_outgoing" => "You whisper to %3: %2",
        "team_msg_command_incoming" => "%3 <%1> %2",
        "team_msg_command_outgoing" => "%3 -> <%1> %2",
        _ => "<%1> %2",
    }
}

fn render_chat(pattern: &str, sender: &str, content: &str, target: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + sender.len() + content.len());
    let mut it = pattern.chars().peekable();
    while let Some(c) = it.next() {
        if c == '%' {
            match it.peek() {
                Some('1') => out.push_str(sender),
                Some('2') => out.push_str(content),
                Some('3') => out.push_str(target),
                _ => {
                    out.push(c);
                    continue;
                }
            }
            it.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// The attributes movement reads (by registry id of `minecraft:attribute`, checked against
/// the 26.1.2 registries report).
fn attribute_name(id: i32) -> Option<&'static str> {
    Some(match id {
        5 => "block_break_speed",
        6 => "block_interaction_range",
        10 => "entity_interaction_range",
        14 => "gravity",
        15 => "jump_strength",
        20 => "mining_efficiency",
        21 => "movement_efficiency",
        22 => "movement_speed",
        24 => "safe_fall_distance",
        25 => "scale",
        26 => "sneaking_speed",
        28 => "step_height",
        29 => "submerged_mining_speed",
        32 => "water_movement_efficiency",
        _ => return None,
    })
}

impl ClientVersion for V26_1 {
    const GAME: GameVersion = GameVersion::V26_1;
    const CHAT_LIMIT: usize = 256;
    type Wire = Wire26_1;

    fn write_handshake(enc: &mut FrameEncoder, host: &str, port: u16) -> Result<(), EncodeError> {
        enc.write_packet(&handshaking::serverbound::SetProtocol {
            protocol_version: 775,
            server_host: ByteString::from(host),
            server_port: port,
            next_state: 2,
        })
    }

    fn write_login_start(
        enc: &mut FrameEncoder,
        username: &str,
        uuid: uuid::Uuid,
    ) -> Result<(), EncodeError> {
        enc.write_packet(&login::serverbound::LoginStart {
            username: ByteString::from(username),
            player_uuid: uuid,
        })
    }

    fn write_encryption_response(
        enc: &mut FrameEncoder,
        shared_secret: &[u8],
        verify_token: &[u8],
    ) -> Result<(), EncodeError> {
        enc.write_packet(&login::serverbound::EncryptionBegin {
            shared_secret: Bytes::copy_from_slice(shared_secret),
            verify_token: Bytes::copy_from_slice(verify_token),
        })
    }

    fn write_plugin_response(enc: &mut FrameEncoder, message_id: i32) -> Result<(), EncodeError> {
        enc.write_packet(&login::serverbound::LoginPluginResponse {
            message_id,
            data: None,
        })
    }

    fn decode_login(raw: &RawPacket) -> Result<Option<LoginInbound>, DecodeError> {
        use login::clientbound as lc;
        Ok(Some(match raw.id {
            lc::id::DISCONNECT => {
                // The login disconnect is still a JSON string.
                LoginInbound::Disconnect(raw.decode_as::<lc::Disconnect>()?.reason)
            }
            lc::id::ENCRYPTION_BEGIN => {
                let p: lc::EncryptionBegin = raw.decode_as()?;
                if p.should_authenticate {
                    LoginInbound::EncryptionRequest {
                        server_id: p.server_id,
                        public_key: p.public_key,
                        verify_token: p.verify_token,
                    }
                } else {
                    LoginInbound::EncryptionRequestUnauthenticated {
                        server_id: p.server_id,
                        public_key: p.public_key,
                        verify_token: p.verify_token,
                    }
                }
            }
            lc::id::SUCCESS => {
                let p: lc::Success = raw.decode_as()?;
                LoginInbound::Success {
                    uuid: Some(p.uuid),
                    username: p.username,
                }
            }
            lc::id::COMPRESS => LoginInbound::Compress(raw.decode_as::<lc::Compress>()?.threshold),
            lc::id::LOGIN_PLUGIN_REQUEST => {
                let p: lc::LoginPluginRequest = raw.decode_as()?;
                LoginInbound::PluginRequest {
                    message_id: p.message_id,
                    channel: p.channel,
                }
            }
            lc::id::COOKIE_REQUEST => {
                LoginInbound::CookieRequest(raw.decode_as::<lc::CookieRequest>()?.cookie)
            }
            _ => return Ok(None),
        }))
    }

    fn prepare_play(out: &[Outbound], wire: &mut Wire26_1) {
        wire.tick_look = out.iter().find_map(|o| match *o {
            Outbound::Move(MovePacket::Look { yaw, pitch, .. })
            | Outbound::Move(MovePacket::PositionLook { yaw, pitch, .. }) => Some((yaw, pitch)),
            _ => None,
        });
    }

    fn write_transfer_handshake(
        enc: &mut FrameEncoder,
        host: &str,
        port: u16,
    ) -> Result<(), EncodeError> {
        // ClientIntent.TRANSFER.
        enc.write_packet(&handshaking::serverbound::SetProtocol {
            protocol_version: 775,
            server_host: ByteString::from(host),
            server_port: port,
            next_state: 3,
        })
    }

    fn take_cookies(wire: &mut Wire26_1) -> Vec<(ByteString, Bytes)> {
        std::mem::take(&mut wire.cookies).into_iter().collect()
    }

    fn put_cookies(wire: &mut Wire26_1, cookies: Vec<(ByteString, Bytes)>) {
        wire.cookies.extend(cookies);
    }

    fn write_login_cookie_response_from(
        enc: &mut FrameEncoder,
        key: &str,
        wire: &mut Wire26_1,
    ) -> Result<(), EncodeError> {
        enc.write_packet(&login::serverbound::CookieResponse {
            key: ByteString::from(key),
            value: wire.cookies.get(key).cloned(),
        })
    }

    fn write_login_cookie_response(enc: &mut FrameEncoder, key: &str) -> Result<(), EncodeError> {
        // A fresh connection has no cookies (they come with a transfer).
        enc.write_packet(&login::serverbound::CookieResponse {
            key: ByteString::from(key),
            value: None,
        })
    }

    fn write_login_acknowledged(enc: &mut FrameEncoder) -> Result<(), EncodeError> {
        enc.write_packet(&login::serverbound::LoginAcknowledged {})
    }

    fn decode_configuration(
        raw: &RawPacket,
        wire: &mut Wire26_1,
    ) -> Result<Option<ConfigInbound>, DecodeError> {
        use configuration::clientbound as cc;
        Ok(Some(match raw.id {
            cc::id::KEEP_ALIVE => {
                ConfigInbound::KeepAlive(raw.decode_as::<cc::KeepAlive>()?.keep_alive_id)
            }
            cc::id::PING => ConfigInbound::Ping(raw.decode_as::<cc::Ping>()?.id),
            cc::id::FINISH_CONFIGURATION => ConfigInbound::Finish,
            cc::id::DISCONNECT => {
                ConfigInbound::Disconnect(plain(&raw.decode_as::<cc::Disconnect>()?.reason))
            }
            cc::id::SELECT_KNOWN_PACKS => {
                let p: cc::SelectKnownPacks = raw.decode_as()?;
                ConfigInbound::SelectKnownPacks(
                    p.packs
                        .into_iter()
                        .map(|k| KnownPack {
                            namespace: k.namespace,
                            id: k.id,
                            version: k.version,
                        })
                        .collect(),
                )
            }
            cc::id::REGISTRY_DATA => ConfigInbound::RegistryData(raw.clone()),
            cc::id::ADD_RESOURCE_PACK => {
                let p: cc::AddResourcePack = raw.decode_as()?;
                ConfigInbound::ResourcePackPush {
                    id: p.uuid,
                    required: p.forced,
                }
            }
            cc::id::COOKIE_REQUEST => {
                ConfigInbound::CookieRequest(raw.decode_as::<cc::CookieRequest>()?.cookie)
            }
            cc::id::STORE_COOKIE => {
                let p: cc::StoreCookie = raw.decode_as()?;
                wire.cookies.insert(p.key, p.value);
                return Ok(None);
            }
            cc::id::CODE_OF_CONDUCT => ConfigInbound::CodeOfConduct(
                raw.decode_as::<cc::CodeOfConduct>()?.contents.to_string(),
            ),
            cc::id::TAGS => {
                wire.inv_config
                    .push((true, Bytes::copy_from_slice(&raw.body)));
                return Ok(None);
            }
            cc::id::TRANSFER => {
                let p: cc::Transfer = raw.decode_as()?;
                ConfigInbound::Transfer(format!("{}:{}", p.host, p.port))
            }
            _ => return Ok(None),
        }))
    }

    fn write_configuration(
        enc: &mut FrameEncoder,
        out: &ConfigOutbound,
        wire: &mut Wire26_1,
    ) -> Result<(), EncodeError> {
        use configuration::serverbound as cs;
        match out {
            ConfigOutbound::KeepAlive(id) => {
                enc.write_packet(&cs::KeepAlive { keep_alive_id: *id })
            }
            ConfigOutbound::Pong(id) => enc.write_packet(&cs::Pong { id: *id }),
            ConfigOutbound::Finish => enc.write_packet(&cs::FinishConfiguration {}),
            ConfigOutbound::SelectKnownPacks(packs) => enc.write_packet(&cs::SelectKnownPacks {
                packs: packs
                    .iter()
                    .map(|k| cs::SelectKnownPacksPacks {
                        namespace: k.namespace.clone(),
                        id: k.id.clone(),
                        version: k.version.clone(),
                    })
                    .collect(),
            }),
            ConfigOutbound::ResourcePackStatus { id, result } => {
                enc.write_packet(&cs::ResourcePackReceive {
                    uuid: *id,
                    result: *result,
                })
            }
            ConfigOutbound::CookieResponse { key } => enc.write_packet(&cs::CookieResponse {
                key: key.clone(),
                value: wire.cookies.get(key).cloned(),
            }),
            ConfigOutbound::AcceptCodeOfConduct => enc.write_packet(&cs::AcceptCodeOfConduct {}),
            ConfigOutbound::Brand(brand) => {
                let mut data = BytesMut::with_capacity(brand.len() + 1);
                data.put_string(brand)?;
                enc.write_packet(&cs::CustomPayload {
                    channel: ByteString::from_static("minecraft:brand"),
                    data: data.freeze(),
                })
            }
            ConfigOutbound::Settings(s) => enc.write_packet(&cs::Settings {
                locale: ByteString::from(s.locale.to_ascii_lowercase()),
                view_distance: s.view_distance,
                chat_flags: i32::from(s.chat_mode),
                chat_colors: s.chat_colors,
                skin_parts: s.skin_parts,
                main_hand: s.main_hand,
                enable_text_filtering: false,
                enable_server_listing: true,
                particle_status: cs::SettingsParticleStatus::ALL,
            }),
        }
    }

    fn feed_inventory_config(inv: &mut ghast_inventory::Inventory, wire: &mut Wire26_1) {
        for (tags, body) in wire.inv_config.drain(..) {
            if tags {
                inv.on_update_tags_26_1(body);
            } else {
                inv.on_registry_data_26_1(body);
            }
        }
    }

    fn mount_screen_entity(raw: &RawPacket) -> Option<i32> {
        if raw.id != cb::id::OPEN_HORSE_WINDOW {
            return None;
        }
        raw.decode_as::<cb::OpenHorseWindow>()
            .ok()
            .map(|p| p.entity_id)
    }

    fn open_mount_screen(
        inv: &mut ghast_inventory::Inventory,
        raw: &RawPacket,
        mount: Option<(i32, bool, bool)>,
    ) -> Result<(), DecodeError> {
        let p: cb::OpenHorseWindow = raw.decode_as()?;
        match mount {
            Some((ty, saddle, body)) => {
                inv.on_mount_screen_26_1(p.window_id, p.nb_slots, ty, saddle, body);
                Ok(())
            }
            // An entity the client does not know (spawned before it could see it): the
            // generic screen.
            None => inv.apply_raw_v26_1(raw),
        }
    }

    fn note_registry(raw: &RawPacket, wire: &mut Wire26_1) {
        let Ok(p) = raw.decode_as::<configuration::clientbound::RegistryData>() else {
            return;
        };
        // The inventory keeps only the enchantment registry (the anvil's rules). A copy: the
        // frame buffer the body points into is not kept alive.
        if &*p.id == "minecraft:enchantment" {
            wire.inv_config
                .push((false, Bytes::copy_from_slice(&raw.body)));
        }
        if &*p.id == "minecraft:chat_type" {
            wire.chat_types = p.entries.iter().map(|e| e.key.to_string()).collect();
        }
        // Every data-driven registry's names (click hashes of components that name entries).
        wire.hash_registries
            .add(&p.id, p.entries.iter().map(|e| &*e.key));
    }

    fn write_configuration_acknowledged(enc: &mut FrameEncoder) -> Result<(), EncodeError> {
        enc.write_packet(&sb::ConfigurationAcknowledged {})
    }

    fn write_configuration_start_replies(
        enc: &mut FrameEncoder,
        wire: &mut Wire26_1,
    ) -> Result<(), EncodeError> {
        // handleConfigurationStart → sendChatAcknowledgement: the offset, if any.
        wire.seen.ack_due = false;
        let count = std::mem::take(&mut wire.seen.offset);
        if count > 0 {
            enc.write_packet(&sb::MessageAcknowledgement { count })?;
        }
        Ok(())
    }

    fn write_wire_replies(enc: &mut FrameEncoder, wire: &mut Wire26_1) -> Result<(), EncodeError> {
        if wire.seen.ack_due {
            wire.seen.ack_due = false;
            let count = std::mem::take(&mut wire.seen.offset);
            if count > 0 {
                enc.write_packet(&sb::MessageAcknowledgement { count })?;
            }
        }
        Ok(())
    }

    fn play_filter(interests: Interests) -> PacketFilter {
        let mut f = PacketFilter::none()
            .with(cb::id::KEEP_ALIVE)
            .with(cb::id::PING)
            .with(cb::id::LOGIN)
            .with(cb::id::UPDATE_HEALTH)
            .with(cb::id::RESPAWN)
            .with(cb::id::POSITION)
            .with(cb::id::PLAYER_ROTATION)
            .with(cb::id::HELD_ITEM_SLOT)
            .with(cb::id::GAME_STATE_CHANGE)
            .with(cb::id::OPEN_WINDOW)
            .with(cb::id::OPEN_HORSE_WINDOW)
            .with(cb::id::CLOSE_WINDOW)
            .with(cb::id::WINDOW_ITEMS)
            .with(cb::id::SET_SLOT)
            .with(cb::id::SET_CURSOR_ITEM)
            .with(cb::id::SET_PLAYER_INVENTORY)
            .with(cb::id::CRAFT_PROGRESS_BAR)
            .with(cb::id::ABILITIES)
            .with(cb::id::KICK_DISCONNECT)
            .with(cb::id::DEATH_COMBAT_EVENT)
            .with(cb::id::ENTITY_STATUS)
            .with(cb::id::ENTITY_UPDATE_ATTRIBUTES)
            .with(cb::id::ENTITY_EFFECT)
            .with(cb::id::REMOVE_ENTITY_EFFECT)
            .with(cb::id::ENTITY_VELOCITY)
            .with(cb::id::EXPLOSION)
            .with(cb::id::START_CONFIGURATION)
            .with(cb::id::ADD_RESOURCE_PACK)
            .with(cb::id::COOKIE_REQUEST)
            .with(cb::id::STORE_COOKIE)
            // Signed player messages must be acknowledged even when nobody reads chat.
            .with(cb::id::PLAYER_CHAT)
            // The command tree decides which commands are signed (kept raw until used).
            .with(cb::id::DECLARE_COMMANDS)
            .with(cb::id::TRANSFER);
        for &id in ghast_world::v26_1::PACKET_IDS {
            f.insert(id);
        }
        // Everything the inventory reads (recipes for the furnace / smithing / stonecutter
        // slots, abilities for creative clicks, experience for the anvil and enchanting).
        for &id in ghast_inventory::PACKET_IDS_26_1 {
            f.insert(id);
        }
        if interests.contains(Interests::CHAT) {
            f.insert(cb::id::SYSTEM_CHAT);
            f.insert(cb::id::PROFILELESS_CHAT);
        }
        f
    }

    fn decode_play(raw: &RawPacket, wire: &mut Wire26_1) -> Result<Option<Inbound>, DecodeError> {
        Ok(Some(match raw.id {
            cb::id::KEEP_ALIVE => {
                Inbound::KeepAlive(raw.decode_as::<cb::KeepAlive>()?.keep_alive_id)
            }
            cb::id::PING => Inbound::Ping(raw.decode_as::<cb::Ping>()?.id),
            cb::id::LOGIN => {
                let p: cb::Login = raw.decode_as()?;
                let w = &p.world_state;
                wire.new_level(&w.name);
                wire.next_chat_index = 0;
                wire.seen = LastSeen::default();
                // handleLogin: no chat session until the key pair is set again; a new
                // LocalPlayer (permission level 0 until the server's entity event).
                wire.chat = None;
                wire.entity_id = p.entity_id;
                wire.op_level = 0;
                Inbound::JoinGame(JoinGame {
                    entity_id: p.entity_id,
                    game_mode: GameMode::from_wire(w.gamemode.0 as u8),
                    dimension: w.name.to_string(),
                    hardcore: p.is_hardcore,
                    dimension_nbt: None,
                    respawn_screen: p.enable_respawn_screen,
                })
            }
            cb::id::RESPAWN => {
                let p: cb::Respawn = raw.decode_as()?;
                let w = &p.world_state;
                if *w.name != *wire.dimension {
                    wire.new_level(&w.name);
                }
                Inbound::Respawn {
                    dimension: w.name.to_string(),
                    game_mode: GameMode::from_wire(w.gamemode.0 as u8),
                    keep: p.copy_metadata,
                }
            }
            cb::id::SYSTEM_CHAT => {
                let p: cb::SystemChat = raw.decode_as()?;
                Inbound::Chat(ChatMessage::from_text(
                    plain(&p.content),
                    if p.is_action_bar {
                        ChatPosition::ActionBar
                    } else {
                        ChatPosition::System
                    },
                    None,
                ))
            }
            cb::id::PLAYER_CHAT => {
                let p: cb::PlayerChat = raw.decode_as()?;
                if p.global_index != wire.next_chat_index {
                    log::warn!(
                        "player chat index {} (expected {}): vanilla disconnects here",
                        p.global_index,
                        wire.next_chat_index
                    );
                }
                wire.next_chat_index = p.global_index.wrapping_add(1);
                if let Some(sig) = p.signature.clone() {
                    wire.seen.add(sig, true);
                }
                let content = p
                    .unsigned_chat_content
                    .as_ref()
                    .map_or_else(|| p.plain_message.to_string(), plain);
                let kind = match &p.r#type {
                    ghast_protocol::types::Holder::Id(i) => usize::try_from(*i)
                        .ok()
                        .and_then(|i| wire.chat_types.get(i))
                        .map_or("minecraft:chat", String::as_str),
                    ghast_protocol::types::Holder::Inline(_) => "minecraft:chat",
                };
                let target = p
                    .network_target_name
                    .as_ref()
                    .map(plain)
                    .unwrap_or_default();
                let text = render_chat(
                    chat_pattern(kind),
                    &plain(&p.network_name),
                    &content,
                    &target,
                );
                Inbound::Chat(ChatMessage::from_text(
                    text,
                    ChatPosition::Chat,
                    Some(p.sender_uuid),
                ))
            }
            cb::id::PROFILELESS_CHAT => {
                let p: cb::ProfilelessChat = raw.decode_as()?;
                let kind = p
                    .r#type
                    .id()
                    .and_then(|i| usize::try_from(i).ok())
                    .and_then(|i| wire.chat_types.get(i))
                    .map_or("minecraft:chat", String::as_str);
                let target = p.target.as_ref().map(plain).unwrap_or_default();
                Inbound::Chat(ChatMessage::from_text(
                    render_chat(
                        chat_pattern(kind),
                        &plain(&p.name),
                        &plain(&p.message),
                        &target,
                    ),
                    ChatPosition::Chat,
                    None,
                ))
            }
            cb::id::UPDATE_HEALTH => {
                let p: cb::UpdateHealth = raw.decode_as()?;
                Inbound::Health(Health {
                    health: p.health,
                    food: p.food,
                    saturation: p.food_saturation,
                })
            }
            cb::id::POSITION => {
                let p: cb::Position = raw.decode_as()?;
                Inbound::Teleport(Teleport {
                    x: p.x,
                    y: p.y,
                    z: p.z,
                    yaw: p.yaw,
                    pitch: p.pitch,
                    flags: (p.flags.0 & 0x1f) as i8,
                    teleport_id: Some(p.teleport_id),
                    relative: p.flags.0,
                    delta: Vec3::new(p.dx, p.dy, p.dz),
                })
            }
            cb::id::PLAYER_ROTATION => {
                let p: cb::PlayerRotation = raw.decode_as()?;
                Inbound::Rotate {
                    yaw: p.yaw,
                    pitch: p.pitch,
                    relative_yaw: p.relative_yaw,
                    relative_pitch: p.relative_pitch,
                }
            }
            cb::id::HELD_ITEM_SLOT => {
                Inbound::HeldSlot(raw.decode_as::<cb::HeldItemSlot>()?.slot as i8)
            }
            cb::id::GAME_STATE_CHANGE => {
                let p: cb::GameStateChange = raw.decode_as()?;
                Inbound::GameState {
                    reason: p.reason.0,
                    value: p.game_mode,
                }
            }
            cb::id::OPEN_WINDOW => {
                Inbound::OpenWindow(raw.decode_as::<cb::OpenWindow>()?.window_id)
            }
            cb::id::CLOSE_WINDOW => {
                Inbound::CloseWindow(raw.decode_as::<cb::CloseWindow>()?.window_id)
            }
            cb::id::ABILITIES => {
                let p: cb::Abilities = raw.decode_as()?;
                Inbound::Abilities(Abilities {
                    flags: p.flags,
                    flying_speed: p.flying_speed,
                    walking_speed: p.walking_speed,
                })
            }
            cb::id::KICK_DISCONNECT => {
                Inbound::Disconnect(reason_json(&raw.decode_as::<cb::KickDisconnect>()?.reason))
            }
            cb::id::ENTITY_VELOCITY => {
                let p: cb::EntityVelocity = raw.decode_as()?;
                let [x, y, z] = p.velocity.to_vec3();
                Inbound::Velocity {
                    entity_id: p.entity_id,
                    motion: Vec3::new(x, y, z),
                }
            }
            cb::id::EXPLOSION => {
                let p: cb::Explosion = raw.decode_as()?;
                let Some(k) = p.player_knockback else {
                    return Ok(None);
                };
                Inbound::ExplosionPush(Vec3::new(k.x, k.y, k.z))
            }
            cb::id::ENTITY_UPDATE_ATTRIBUTES => {
                let p: cb::EntityUpdateAttributes = raw.decode_as()?;
                let attributes: Vec<AttributeUpdate> = p
                    .properties
                    .iter()
                    .filter_map(|a| {
                        Some(AttributeUpdate {
                            name: attribute_name(a.key.0)?,
                            base: a.value,
                            modifiers: a
                                .modifiers
                                .iter()
                                .map(|m| (m.uuid.clone(), m.amount, m.operation))
                                .collect(),
                        })
                    })
                    .collect();
                if attributes.is_empty() {
                    return Ok(None);
                }
                Inbound::Attributes {
                    entity_id: p.entity_id,
                    attributes,
                }
            }
            cb::id::ENTITY_EFFECT => {
                let p: cb::EntityEffect = raw.decode_as()?;
                // Registry ids are 0-based; the core uses the classic 1-based effect numbers.
                Inbound::Effect {
                    entity_id: p.entity_id,
                    id: i8::try_from(p.effect_id + 1).unwrap_or(i8::MAX),
                    amplifier: p.amplifier.clamp(0, 127) as i8,
                }
            }
            cb::id::REMOVE_ENTITY_EFFECT => {
                let p: cb::RemoveEntityEffect = raw.decode_as()?;
                Inbound::EffectRemoved {
                    entity_id: p.entity_id,
                    id: i8::try_from(p.effect_id + 1).unwrap_or(i8::MAX),
                }
            }
            cb::id::ENTITY_STATUS => {
                let p: cb::EntityStatus = raw.decode_as()?;
                if p.entity_id == wire.entity_id && (24..=28).contains(&p.entity_status) {
                    wire.op_level = (p.entity_status - 24) as u8;
                }
                Inbound::EntityStatus {
                    entity_id: p.entity_id,
                    status: p.entity_status,
                }
            }
            cb::id::DEATH_COMBAT_EVENT => Inbound::Died {
                entity_id: raw.decode_as::<cb::DeathCombatEvent>()?.player_id,
            },
            cb::id::CHUNK_BATCH_START => Inbound::ChunkBatchStart,
            cb::id::CHUNK_BATCH_FINISHED => {
                Inbound::ChunkBatchFinished(raw.decode_as::<cb::ChunkBatchFinished>()?.batch_size)
            }
            cb::id::START_CONFIGURATION => Inbound::StartConfiguration,
            cb::id::ADD_RESOURCE_PACK => {
                let p: cb::AddResourcePack = raw.decode_as()?;
                Inbound::ResourcePackPush {
                    id: p.uuid,
                    required: p.forced,
                }
            }
            cb::id::COOKIE_REQUEST => {
                Inbound::CookieRequest(raw.decode_as::<cb::CookieRequest>()?.cookie)
            }
            cb::id::STORE_COOKIE => {
                let p: cb::StoreCookie = raw.decode_as()?;
                wire.cookies.insert(p.key, p.value);
                return Ok(None);
            }
            cb::id::TRANSFER => {
                let p: cb::Transfer = raw.decode_as()?;
                Inbound::Transfer {
                    host: p.host.to_string(),
                    port: u16::try_from(p.port).unwrap_or(25565),
                }
            }
            cb::id::DECLARE_COMMANDS => {
                // Kept raw: parsed only when a command is sent.
                wire.commands_raw = Some(deflate(raw));
                wire.commands = None;
                return Ok(None);
            }
            _ => return Ok(None),
        }))
    }

    fn begin_chat_session(
        wire: &mut Wire26_1,
        keys: &std::sync::Arc<ghast_auth::chat::ProfileKeys>,
        profile_id: uuid::Uuid,
    ) -> bool {
        // setKeyPair: a new session unless the same key pair is already in use.
        if wire
            .chat
            .as_ref()
            .is_some_and(|s| std::sync::Arc::ptr_eq(&s.keys, keys))
        {
            return false;
        }
        wire.chat = Some(ChatSession {
            keys: keys.clone(),
            chain: ghast_auth::chat::SignedMessageChain::new(profile_id, random_uuid()),
        });
        true
    }

    const INVENTORY: ghast_inventory::Version = ghast_inventory::Version::V26_1;

    fn inventory_packet(
        inv: &mut ghast_inventory::Inventory,
        raw: &RawPacket,
    ) -> Result<Option<ghast_inventory::Outgoing>, DecodeError> {
        // Slots keep their components as bytes (structural skip); no confirms since 1.17.
        if (raw.id == cb::id::LOGIN
            || raw.id == cb::id::RESPAWN
            || ghast_inventory::PACKET_IDS_26_1.contains(&raw.id))
            // The mount screen needs the entity (ClientVersion::open_mount_screen).
            && raw.id != cb::id::OPEN_HORSE_WINDOW
        {
            inv.apply_raw_v26_1(raw)?;
            // CreativeInventoryListener.slotChanged → handleCreativeModeItemAdd: the open creative
            // screen answers each window 0 slot the server sets, while the packet is handled.
            let echo = inv.take_creative_echo_26_1();
            if !echo.is_empty() {
                return Ok(Some(ghast_inventory::Outgoing::Creative(echo)));
            }
        }
        Ok(None)
    }

    fn configure_inventory(inv: &mut ghast_inventory::Inventory, wire: &Wire26_1) {
        inv.set_registries_26_1(wire.hash_registries.clone());
    }

    fn write_play(
        enc: &mut FrameEncoder,
        out: &Outbound,
        wire: &mut Wire26_1,
    ) -> Result<(), EncodeError> {
        let flags = |on_ground: bool, horizontal_collision: bool| {
            let mut f = sb::MovementFlags(0);
            if on_ground {
                f = f.with(sb::MovementFlags::ON_GROUND);
            }
            if horizontal_collision {
                f = f.with(sb::MovementFlags::HAS_HORIZONTAL_COLLISION);
            }
            f
        };
        match out {
            Outbound::KeepAlive(id) => enc.write_packet(&sb::KeepAlive { keep_alive_id: *id }),
            Outbound::TeleportConfirm(teleport_id) => enc.write_packet(&sb::TeleportConfirm {
                teleport_id: *teleport_id,
            }),
            Outbound::Move(m) => match *m {
                MovePacket::Flying {
                    on_ground,
                    horizontal_collision,
                } => enc.write_packet(&sb::Flying {
                    flags: flags(on_ground, horizontal_collision),
                }),
                MovePacket::Position {
                    pos,
                    on_ground,
                    horizontal_collision,
                } => enc.write_packet(&sb::Position {
                    x: pos.x,
                    y: pos.y,
                    z: pos.z,
                    flags: flags(on_ground, horizontal_collision),
                }),
                MovePacket::Look {
                    yaw,
                    pitch,
                    on_ground,
                    horizontal_collision,
                } => {
                    wire.last_look = (yaw, pitch);
                    enc.write_packet(&sb::Look {
                        yaw,
                        pitch,
                        flags: flags(on_ground, horizontal_collision),
                    })
                }
                MovePacket::PositionLook {
                    pos,
                    yaw,
                    pitch,
                    on_ground,
                    horizontal_collision,
                } => {
                    wire.last_look = (yaw, pitch);
                    enc.write_packet(&sb::PositionLook {
                        x: pos.x,
                        y: pos.y,
                        z: pos.z,
                        yaw,
                        pitch,
                        flags: flags(on_ground, horizontal_collision),
                    })
                }
            },
            Outbound::VehicleMove {
                pos,
                yaw,
                pitch,
                on_ground,
            } => enc.write_packet(&sb::VehicleMove {
                x: pos.x,
                y: pos.y,
                z: pos.z,
                yaw: *yaw,
                pitch: *pitch,
                on_ground: *on_ground,
            }),
            Outbound::SteerBoat { left, right } => enc.write_packet(&sb::SteerBoat {
                left_paddle: *left,
                right_paddle: *right,
            }),
            Outbound::LookAnswer { yaw, pitch } => {
                wire.last_look = (*yaw, *pitch);
                enc.write_packet(&sb::Look {
                    yaw: *yaw,
                    pitch: *pitch,
                    flags: flags(false, false),
                })
            }
            Outbound::EntityAction { entity_id, action } => {
                let action_id = match action {
                    EntityAction::StartSprinting => sb::EntityActionActionId::START_SPRINTING,
                    EntityAction::StopSprinting => sb::EntityActionActionId::STOP_SPRINTING,
                    EntityAction::StartFallFlying => sb::EntityActionActionId::START_FALL_FLYING,
                    // Sneaking is part of `player_input` since 1.21.2.
                    _ => return Ok(()),
                };
                enc.write_packet(&sb::EntityAction {
                    entity_id: *entity_id,
                    action_id,
                    jump_boost: 0,
                })
            }
            Outbound::PlayerInput(c) => {
                let mut i = sb::PlayerInputInputs(0);
                for (on, bit) in [
                    (c.forward, sb::PlayerInputInputs::FORWARD),
                    (c.back, sb::PlayerInputInputs::BACKWARD),
                    (c.left, sb::PlayerInputInputs::LEFT),
                    (c.right, sb::PlayerInputInputs::RIGHT),
                    (c.jump, sb::PlayerInputInputs::JUMP),
                    (c.sneak, sb::PlayerInputInputs::SHIFT),
                    (c.sprint, sb::PlayerInputInputs::SPRINT),
                ] {
                    if on {
                        i = sb::PlayerInputInputs(i.0 | bit.0);
                    }
                }
                enc.write_packet(&sb::PlayerInput { inputs: i })
            }
            Outbound::TickEnd => enc.write_packet(&sb::TickEnd {}),
            Outbound::PlayerLoaded => enc.write_packet(&sb::PlayerLoaded {}),
            Outbound::ChunkBatchReceived(chunks_per_tick) => {
                enc.write_packet(&sb::ChunkBatchReceived {
                    chunks_per_tick: *chunks_per_tick,
                })
            }
            Outbound::Pong(id) => enc.write_packet(&sb::Pong { id: *id }),
            Outbound::Chat(text) => match text.strip_prefix('/') {
                Some(command) => wire.write_command(enc, command),
                None => wire.write_chat(enc, text),
            },
            Outbound::ChatSessionUpdate => match &wire.chat {
                Some(s) => enc.write_packet(&sb::ChatSessionUpdate {
                    session_uuid: s.chain.session_id(),
                    expire_time: s.keys.expires_at_millis(),
                    public_key: Bytes::copy_from_slice(s.keys.public_key_der()),
                    signature: Bytes::copy_from_slice(s.keys.key_signature()),
                }),
                None => Ok(()),
            },
            Outbound::Respawn => enc.write_packet(&sb::ClientCommand {
                action_id: sb::ClientCommandActionId::PERFORM_RESPAWN,
            }),
            Outbound::Abilities { flags, .. } => enc.write_packet(&sb::Abilities { flags: *flags }),
            Outbound::HeldSlot(slot) => enc.write_packet(&sb::HeldItemSlot { slot_id: *slot }),
            Outbound::Swing => enc.write_packet(&sb::ArmAnimation {
                hand: sb::ArmAnimationHand::MAIN_HAND,
            }),
            Outbound::SwingOffHand => enc.write_packet(&sb::ArmAnimation {
                hand: sb::ArmAnimationHand::OFF_HAND,
            }),
            Outbound::Dig { status, pos, face } => {
                use crate::common::DigStatus;
                // Start and finish are predictions (a new sequence); abort and release are not.
                let sequence = match status {
                    DigStatus::Start | DigStatus::Finish => wire.next_sequence(),
                    _ => 0,
                };
                enc.write_packet(&sb::BlockDig {
                    status: status.id(),
                    location: *pos,
                    face: face.id(),
                    sequence,
                })
            }
            Outbound::Place {
                pos,
                face,
                cursor,
                off_hand,
                ..
            } => {
                let sequence = wire.next_sequence();
                enc.write_packet(&sb::BlockPlace {
                    hand: sb::BlockPlaceHand(i32::from(*off_hand)),
                    location: *pos,
                    direction: i32::from(face.id()),
                    cursor_x: cursor.x as f32,
                    cursor_y: cursor.y as f32,
                    cursor_z: cursor.z as f32,
                    inside_block: false,
                    world_border_hit: false,
                    sequence,
                })
            }
            Outbound::UseItem { off_hand, .. } => {
                let sequence = wire.next_sequence();
                let (yaw, pitch) = wire.tick_look.unwrap_or(wire.last_look);
                enc.write_packet(&sb::UseItem {
                    hand: sb::UseItemHand(i32::from(*off_hand)),
                    sequence,
                    rotation: ghast_protocol::v26_1::types::Vec2f { x: yaw, y: pitch },
                })
            }
            Outbound::UseEntity {
                target,
                action,
                off_hand,
                sneaking,
            } => match action {
                crate::common::EntityUse::Attack => {
                    enc.write_packet(&sb::Attack { entity_id: *target })
                }
                crate::common::EntityUse::InteractAt(v) => enc.write_packet(&sb::UseEntity {
                    target: *target,
                    hand: sb::UseEntityHand(i32::from(*off_hand)),
                    location: LpVec3::from_vec3([v.x, v.y, v.z]),
                    sneaking: *sneaking,
                }),
                // 26.1 has one interaction packet, sent with its hit location (`InteractAt`).
                _ => Ok(()),
            },
            Outbound::Settings(s) => enc.write_packet(&sb::Settings {
                locale: ByteString::from(s.locale.to_ascii_lowercase()),
                view_distance: s.view_distance,
                chat_flags: i32::from(s.chat_mode),
                chat_colors: s.chat_colors,
                skin_parts: s.skin_parts,
                main_hand: s.main_hand,
                enable_text_filtering: false,
                enable_server_listing: true,
                particle_status: sb::SettingsParticleStatus::ALL,
            }),
            Outbound::Brand(brand) => {
                let mut data = BytesMut::with_capacity(brand.len() + 1);
                data.put_string(brand)?;
                enc.write_packet(&sb::CustomPayload {
                    channel: ByteString::from_static("minecraft:brand"),
                    data: data.freeze(),
                })
            }
            Outbound::ResourcePackStatus { id, result } => {
                enc.write_packet(&sb::ResourcePackReceive {
                    uuid: *id,
                    result: *result,
                })
            }
            Outbound::CookieResponse { key, value } => enc.write_packet(&sb::CookieResponse {
                key: key.clone(),
                value: value.clone().or_else(|| wire.cookies.get(key).cloned()),
            }),
            // Only the window packets' encoders (write_set would link every packet's).
            Outbound::Inventory(o) => match o.to_v26_1(&wire.hash_registries) {
                Some(sb::ServerboundPlayPacket::WindowClick(p)) => {
                    // Every stack the click drops swings the arm first (ServerboundSwingPacket
                    // from LocalPlayer.drop / the slot's throw), then the click goes out.
                    if let ghast_inventory::Outgoing::ContainerClick(c) = o {
                        for _ in 0..c.swings {
                            enc.write_packet(&sb::ArmAnimation {
                                hand: sb::ArmAnimationHand::MAIN_HAND,
                            })?;
                        }
                    }
                    enc.write_packet(&p)
                }
                Some(sb::ServerboundPlayPacket::CloseWindow(p)) => enc.write_packet(&p),
                Some(sb::ServerboundPlayPacket::NameItem(p)) => enc.write_packet(&p),
                Some(sb::ServerboundPlayPacket::EnchantItem(p)) => enc.write_packet(&p),
                Some(sb::ServerboundPlayPacket::SelectTrade(p)) => enc.write_packet(&p),
                None => match o {
                    // The creative screen: swings and set_creative_mode_slot, in order.
                    ghast_inventory::Outgoing::Creative(list) => {
                        for c in list {
                            match c.to_v26_1() {
                                sb::ServerboundPlayPacket::ArmAnimation(p) => {
                                    enc.write_packet(&p)?
                                }
                                sb::ServerboundPlayPacket::SetCreativeSlot(p) => {
                                    enc.write_packet(&p)?;
                                }
                                _ => {}
                            }
                        }
                        Ok(())
                    }
                    _ => Ok(()),
                },
                _ => Ok(()),
            },
            // No transactions since 1.17.
            Outbound::Transaction { .. } => Ok(()),
        }
    }

    fn apply_teleport(pose: &mut Pose, t: &Teleport) {
        // The physics applies 26.1 teleports (relative deltas included); this keeps the
        // position / rotation rules for callers without a world.
        let (yaw, pitch) = teleport_target(pose, t);
        pose.yaw = yaw;
        pose.pitch = pitch.clamp(-90.0, 90.0);
    }
}

impl super::HasSim for V26_1 {
    type Sim = crate::sim::Sim26;
    fn new_sim(opts: &crate::sim::SimOptions) -> Self::Sim {
        crate::sim::Sim26::with(opts)
    }
}

#[cfg(test)]
#[path = "v26_1_chat_golden.rs"]
mod chat_golden;

/// A packet kept for later as one compressed frame (the protocol's own deflate).
fn deflate(raw: &RawPacket) -> Box<[u8]> {
    let mut enc = FrameEncoder::new();
    enc.set_compression(0);
    match enc.write_raw(raw) {
        Ok(()) => enc.pending().into(),
        Err(_) => Box::default(),
    }
}

/// The packet [`deflate`] kept (`None` when corrupt: the tree then stays unknown).
fn inflate(z: &[u8]) -> Option<RawPacket> {
    let mut dec = ghast_protocol::FrameDecoder::new();
    dec.set_compression(0);
    dec.push(z);
    dec.next_packet().ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_seen_checksum_and_bits() {
        let mut t = LastSeen::default();
        // Nothing seen: checksum 1, no bits.
        assert_eq!(t.update(), (0, [0; 3], 1));
        t.add(Bytes::from_static(&[1, 2, 3]), true);
        t.add(Bytes::from_static(&[1, 2, 3]), true); // the same message again is ignored
        t.add(Bytes::from_static(&[200]), true);
        let (offset, bits, checksum) = t.update();
        assert_eq!(offset, 2);
        // The ring starts at the tail: the two entries are the last two slots.
        assert_eq!(bits, [0, 0, 0b1100]);
        let h1 = java_array_hash(&[1, 2, 3]);
        let h2 = java_array_hash(&[200]);
        // checksum = 1; then 31 * checksum + hash per entry, oldest first.
        let c = 31i32.wrapping_add(h1).wrapping_mul(31).wrapping_add(h2);
        assert_eq!(checksum, if c as u8 == 0 { 1 } else { c as u8 });
        // Arrays.hashCode([1,2,3]) = 30817.
        assert_eq!(h1, 30817);
    }

    /// The creative screen: a drop click writes its swing and set_creative_mode_slot packets;
    /// a slot the server sets while the screen is open is echoed back at once.
    #[test]
    fn creative_screen_packets_and_echo() {
        use ghast_inventory::{Click, Inventory, Outgoing, Stack, Version};
        let mut inv = Inventory::new(Version::V26_1).unwrap();
        inv.set_creative_26_1(true);
        let stone = ghast_inventory::v26_1::item_id("stone").unwrap();
        let mut w0 = vec![None; 46];
        w0[36] = Some(Stack::new(stone, 64, 0));
        inv.on_window_items_26_1(0, 1, w0, None);
        inv.tick();
        let open = inv.open_inventory().unwrap();
        assert!(matches!(open, Some(Outgoing::Creative(_))));
        inv.tick();
        let o = inv
            .click(Click::Drop {
                slot: 36,
                all: false,
            })
            .unwrap();
        let mut enc = FrameEncoder::new();
        let mut wire = Wire26_1::default();
        V26_1::write_play(&mut enc, &Outbound::Inventory(o), &mut wire).unwrap();
        let ids = frame_ids(&mut enc);
        assert_eq!(ids[0], sb::id::ARM_ANIMATION);
        assert!(
            ids[1..].iter().all(|&i| i == sb::id::SET_CREATIVE_SLOT),
            "{ids:?}"
        );
        assert!(ids.len() >= 2);
        // The server sets window 0 slot 37 (5 stone): the screen echoes it.
        let mut body = vec![0x00, 0x02, 0x00, 37, 0x05];
        let mut id = stone as u32;
        loop {
            let b = (id & 0x7f) as u8;
            id >>= 7;
            if id == 0 {
                body.push(b);
                break;
            }
            body.push(b | 0x80);
        }
        body.extend_from_slice(&[0, 0]);
        let raw = RawPacket::new(cb::id::SET_SLOT, Bytes::from(body));
        let echo = V26_1::inventory_packet(&mut inv, &raw).unwrap();
        let Some(Outgoing::Creative(list)) = echo else {
            panic!("no echo: {echo:?}");
        };
        // The Q drop did not broadcast (CreativeModeInventoryScreen.slotClicked THROW), so the
        // set_slot's broadcast re-sends slot 36 before the server's slot 37, like vanilla.
        let slots: Vec<(i16, i32)> = list
            .iter()
            .map(|c| match c {
                ghast_inventory::CreativePacket::SetSlot { slot, item } => {
                    (*slot, item.as_ref().map_or(0, |s| s.count()))
                }
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(slots, vec![(36, 63), (37, 5)]);
        let n = list.len();
        V26_1::write_play(
            &mut enc,
            &Outbound::Inventory(Outgoing::Creative(list)),
            &mut wire,
        )
        .unwrap();
        assert_eq!(frame_ids(&mut enc), vec![sb::id::SET_CREATIVE_SLOT; n]);
    }

    /// Packet ids of the frames written (no compression).
    fn frame_ids(enc: &mut FrameEncoder) -> Vec<i32> {
        let out = enc.out_mut().split().freeze();
        let mut v = Vec::new();
        let mut r = &out[..];
        while !r.is_empty() {
            let (len, rest) = (usize::from(r[0]), &r[1..]);
            assert!(r[0] < 0x80, "long frame");
            v.push(i32::from(rest[0]));
            r = &rest[len..];
        }
        v
    }

    /// The menu registry id of a window kind.
    fn menu_of(kind: ghast_inventory::WindowKind) -> i32 {
        (0..64)
            .find(|&m| ghast_inventory::v26_1::window_kind(m) == Some(kind))
            .unwrap()
    }

    /// An anvil rename and a trade selection are written as `rename_item` / `select_trade`.
    #[test]
    fn anvil_rename_and_trade_packets() {
        use ghast_inventory::{Inventory, Stack, Version, WindowKind};
        let mut wire = Wire26_1::default();
        let mut enc = FrameEncoder::new();
        let mut inv = Inventory::new(Version::V26_1).unwrap();
        inv.on_open_screen_26_1(
            3,
            menu_of(WindowKind::Anvil),
            ByteString::from_static("Anvil"),
        );
        let sword = ghast_inventory::v26_1::item_id("iron_sword").unwrap();
        inv.on_set_slot(3, 0, Some(Stack::new(sword, 1, 0)));
        inv.tick();
        let o = inv.anvil_rename_26_1("Blade").unwrap().expect("a rename");
        V26_1::write_play(&mut enc, &Outbound::Inventory(o), &mut wire).unwrap();
        assert_eq!(frame_ids(&mut enc), vec![sb::id::NAME_ITEM]);
        // The same name again sends nothing.
        assert!(inv.anvil_rename_26_1("Blade").unwrap().is_none());
        let mut inv = Inventory::new(Version::V26_1).unwrap();
        inv.on_open_screen_26_1(
            4,
            menu_of(WindowKind::Merchant),
            ByteString::from_static("Villager"),
        );
        inv.tick();
        let o = inv.select_trade_26_1(1).unwrap();
        V26_1::write_play(&mut enc, &Outbound::Inventory(o), &mut wire).unwrap();
        assert_eq!(frame_ids(&mut enc), vec![sb::id::SELECT_TRADE]);
    }

    /// A mount screen opens with what the client knows about the entity: a tamed adult horse
    /// (saddle + body), a baby horse (body only), a llama (both), an unknown entity (the
    /// generic screen).
    #[test]
    fn mount_screen_reads_the_entity() {
        use crate::sim::Sim;
        use ghast_inventory::{Inventory, Version, WindowKind};
        use ghast_protocol::v26_1::types::{
            EntityMetadataEntry, EntityMetadataEntryType as T, EntityMetadataEntryValue as V,
        };
        fn body<P: ghast_protocol::Packet + ghast_protocol::Encode>(p: &P) -> Bytes {
            let mut enc = FrameEncoder::new();
            enc.write_packet(p).unwrap();
            let out = enc.out_mut().split().freeze();
            // one-byte length and id here
            Bytes::copy_from_slice(&out[2..])
        }
        let mut sim = crate::sim::Sim26::test_overworld();
        let spawn = |id: i32, ty: i32| {
            RawPacket::new(
                cb::id::SPAWN_ENTITY,
                body(&cb::SpawnEntity {
                    entity_id: id,
                    object_uuid: uuid::Uuid::from_u64_pair(1, id as u64),
                    r#type: ty,
                    x: 0.5,
                    y: 4.0,
                    z: 0.5,
                    velocity: LpVec3::ZERO,
                    pitch: 0,
                    yaw: 0,
                    head_pitch: 0,
                    object_data: 0,
                }),
            )
        };
        let meta = |id: i32, key: u8, ty: T, value: V| {
            RawPacket::new(
                cb::id::ENTITY_METADATA,
                body(&cb::EntityMetadata {
                    entity_id: id,
                    metadata: vec![EntityMetadataEntry {
                        key,
                        r#type: ty,
                        value,
                    }],
                }),
            )
        };
        sim.world_packet(&spawn(50, 66)).unwrap(); // horse
        assert_eq!(sim.mount_info(50), Some((66, false, true)));
        sim.world_packet(&meta(50, 18, T::BYTE, V::Byte(2)))
            .unwrap(); // tamed
        assert_eq!(sim.mount_info(50), Some((66, true, true)));
        sim.world_packet(&meta(50, 16, T::BOOLEAN, V::Boolean(true)))
            .unwrap(); // baby
        assert_eq!(sim.mount_info(50), Some((66, false, true)));
        sim.world_packet(&spawn(51, 78)).unwrap(); // llama
        assert_eq!(sim.mount_info(51), Some((78, true, true)));
        sim.world_packet(&spawn(52, 88)).unwrap(); // nautilus, not tame
        assert_eq!(sim.mount_info(52), Some((88, false, false)));
        let destroy = RawPacket::new(
            cb::id::ENTITY_DESTROY,
            body(&cb::EntityDestroy {
                entity_ids: vec![51],
            }),
        );
        sim.world_packet(&destroy).unwrap();
        assert_eq!(sim.mount_info(51), None);
        // The screen packet: window 5, 3 columns, entity 50.
        let open = RawPacket::new(
            cb::id::OPEN_HORSE_WINDOW,
            body(&cb::OpenHorseWindow {
                window_id: 5,
                nb_slots: 3,
                entity_id: 50,
            }),
        );
        assert_eq!(V26_1::mount_screen_entity(&open), Some(50));
        let mut inv = Inventory::new(Version::V26_1).unwrap();
        V26_1::open_mount_screen(&mut inv, &open, sim.mount_info(50)).unwrap();
        let w = inv.window().expect("mount screen");
        assert_eq!(w.id(), 5);
        assert!(
            matches!(w.kind(), WindowKind::Horse { .. }),
            "{:?}",
            w.kind()
        );
    }

    /// The play filter passes every packet the inventory reads.
    #[test]
    fn filter_passes_the_inventory_packets() {
        let f = V26_1::play_filter(Interests::NONE);
        for &id in ghast_inventory::PACKET_IDS_26_1 {
            assert!(f.contains(id), "0x{id:02x}");
        }
    }

    /// A click that drops a stack (Q over a filled slot) sends its swings before the click.
    #[test]
    fn dropping_click_swings_first() {
        use ghast_inventory::{Click, Inventory, Stack, Version};
        let mut inv = Inventory::new(Version::V26_1).unwrap();
        let dirt = ghast_inventory::v26_1::item_id("dirt").unwrap();
        inv.on_set_slot(0, 36, Some(Stack::new(dirt, 5, 0)));
        let _ = inv.open_inventory().unwrap();
        // Clicks wait for the tick after the screen opened / the slot changed.
        for _ in 0..3 {
            inv.tick();
        }
        let o = inv
            .click(Click::Drop {
                slot: 36,
                all: false,
            })
            .unwrap();
        let swings = match &o {
            ghast_inventory::Outgoing::ContainerClick(c) => c.swings,
            other => panic!("{other:?}"),
        };
        assert_eq!(swings, 1);
        let mut enc = FrameEncoder::new();
        let mut wire = Wire26_1::default();
        V26_1::write_play(&mut enc, &Outbound::Inventory(o), &mut wire).unwrap();
        let out = enc.out_mut().split().freeze();
        // frame 1: arm_animation (len 2: id, hand 0); frame 2: the window click.
        assert_eq!(&out[..3], &[2, sb::id::ARM_ANIMATION as u8, 0]);
        assert_eq!(out[4], sb::id::WINDOW_CLICK as u8);
    }

    /// A transfer reconnects with `ClientIntent.TRANSFER` (3).
    #[test]
    fn transfer_handshake_has_the_transfer_intent() {
        let mut enc = FrameEncoder::new();
        V26_1::write_transfer_handshake(&mut enc, "example.org", 25566).unwrap();
        let out = enc.out_mut().split().freeze();
        // length, id 0, protocol 775 (varint 0x87 0x06), host, port, next state.
        let host = b"example.org";
        let mut want = vec![0x00, 0x87, 0x06, host.len() as u8];
        want.extend_from_slice(host);
        want.extend_from_slice(&25566u16.to_be_bytes());
        want.push(3);
        assert_eq!(out[0] as usize, want.len());
        assert_eq!(&out[1..], &want[..]);
    }

    #[test]
    fn chat_rendering() {
        assert_eq!(
            render_chat(chat_pattern("minecraft:chat"), "Steve", "hi", ""),
            "<Steve> hi"
        );
        assert_eq!(
            render_chat(chat_pattern("minecraft:say_command"), "Server", "%2 x", ""),
            "[Server] %2 x"
        );
    }
}
