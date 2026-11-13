//! Protocol 340 (1.12.2), the first member of the 1.9 – 1.12 family supported here.

use bytes::BytesMut;
use ghast_protocol::types::ByteString;
use ghast_protocol::v1_12_2::{handshaking, login, play};
use ghast_protocol::{DecodeError, EncodeError, FrameEncoder, McWrite, PacketFilter, RawPacket};

use super::{ClientVersion, GameVersion, V1_12_2, teleport_target, truncate_utf16};
use crate::common::{
    Abilities, ChatMessage, ChatPosition, EntityAction, GameMode, Health, Inbound, JoinGame,
    LoginInbound, MovePacket, Outbound, Pose, Teleport, Vec3,
};
use crate::interest::Interests;

use play::clientbound as cb;
use play::serverbound as sb;

/// `combat_event` event id for "entity dead".
const COMBAT_ENTITY_DEAD: i32 = 2;

impl ClientVersion for V1_12_2 {
    const GAME: GameVersion = GameVersion::V1_12_2;
    const CHAT_LIMIT: usize = 256;

    fn write_handshake(enc: &mut FrameEncoder, host: &str, port: u16) -> Result<(), EncodeError> {
        enc.write_packet(&handshaking::serverbound::SetProtocol {
            protocol_version: 340,
            server_host: ByteString::from(host),
            server_port: port,
            next_state: 2,
        })
    }

    type Wire = ();

    fn write_login_start(
        enc: &mut FrameEncoder,
        username: &str,
        _uuid: uuid::Uuid,
    ) -> Result<(), EncodeError> {
        enc.write_packet(&login::serverbound::LoginStart {
            username: ByteString::from(username),
        })
    }

    fn write_encryption_response(
        enc: &mut FrameEncoder,
        shared_secret: &[u8],
        verify_token: &[u8],
    ) -> Result<(), EncodeError> {
        enc.write_packet(&login::serverbound::EncryptionBegin {
            shared_secret: bytes::Bytes::copy_from_slice(shared_secret),
            verify_token: bytes::Bytes::copy_from_slice(verify_token),
        })
    }

    fn write_plugin_response(_enc: &mut FrameEncoder, _message_id: i32) -> Result<(), EncodeError> {
        // 1.12 has no login plugin messages; decode_login never asks for an answer.
        Ok(())
    }

    fn decode_login(raw: &RawPacket) -> Result<Option<LoginInbound>, DecodeError> {
        use login::clientbound as lc;
        Ok(Some(match raw.id {
            lc::id::DISCONNECT => {
                LoginInbound::Disconnect(raw.decode_as::<lc::Disconnect>()?.reason)
            }
            lc::id::ENCRYPTION_BEGIN => {
                let p: lc::EncryptionBegin = raw.decode_as()?;
                LoginInbound::EncryptionRequest {
                    server_id: p.server_id,
                    public_key: p.public_key,
                    verify_token: p.verify_token,
                }
            }
            lc::id::SUCCESS => {
                let p: lc::Success = raw.decode_as()?;
                LoginInbound::Success {
                    uuid: uuid::Uuid::parse_str(&p.uuid).ok(),
                    username: p.username,
                }
            }
            lc::id::COMPRESS => LoginInbound::Compress(raw.decode_as::<lc::Compress>()?.threshold),
            _ => return Ok(None),
        }))
    }

    fn play_filter(interests: Interests) -> PacketFilter {
        let mut f = PacketFilter::none()
            .with(cb::id::KEEP_ALIVE)
            .with(cb::id::LOGIN)
            .with(cb::id::UPDATE_HEALTH)
            .with(cb::id::RESPAWN)
            .with(cb::id::POSITION)
            .with(cb::id::HELD_ITEM_SLOT)
            .with(cb::id::GAME_STATE_CHANGE)
            .with(cb::id::OPEN_WINDOW)
            .with(cb::id::CLOSE_WINDOW)
            .with(cb::id::TRANSACTION)
            .with(cb::id::WINDOW_ITEMS)
            .with(cb::id::SET_SLOT)
            .with(cb::id::CRAFT_PROGRESS_BAR)
            .with(cb::id::ABILITIES)
            .with(cb::id::KICK_DISCONNECT)
            .with(cb::id::COMBAT_EVENT)
            .with(cb::id::ENTITY_STATUS)
            // Our speed attribute and status effects (physics, dig speed).
            .with(cb::id::ENTITY_UPDATE_ATTRIBUTES)
            .with(cb::id::ENTITY_EFFECT)
            .with(cb::id::REMOVE_ENTITY_EFFECT)
            // Knockback (every entity's velocity passes the id filter; only ours is applied).
            .with(cb::id::ENTITY_VELOCITY);
        // The world: chunks, block changes, border (a 1.9+ client player is not ticked until
        // its column has arrived).
        for &id in ghast_world::v1_12::PACKET_IDS {
            f.insert(id);
        }
        if interests.contains(Interests::CHAT) {
            f.insert(cb::id::CHAT);
        }
        f
    }

    fn decode_play(raw: &RawPacket, (): &mut ()) -> Result<Option<Inbound>, DecodeError> {
        Ok(Some(match raw.id {
            cb::id::KEEP_ALIVE => {
                Inbound::KeepAlive(raw.decode_as::<cb::KeepAlive>()?.keep_alive_id)
            }
            cb::id::LOGIN => {
                let p: cb::Login = raw.decode_as()?;
                Inbound::JoinGame(JoinGame {
                    entity_id: p.entity_id,
                    game_mode: GameMode::from_wire(p.game_mode),
                    dimension: p.dimension.to_string(),
                    hardcore: p.game_mode & 0x08 != 0,
                    dimension_nbt: None,
                    respawn_screen: true,
                })
            }
            cb::id::CHAT => {
                let p: cb::Chat = raw.decode_as()?;
                Inbound::Chat(ChatMessage {
                    rendered: None,
                    json: p.message,
                    position: ChatPosition::from_wire(p.position),
                    sender: None,
                })
            }
            cb::id::UPDATE_HEALTH => {
                let p: cb::UpdateHealth = raw.decode_as()?;
                Inbound::Health(Health {
                    health: p.health,
                    food: p.food,
                    saturation: p.food_saturation,
                })
            }
            cb::id::RESPAWN => {
                let p: cb::Respawn = raw.decode_as()?;
                Inbound::Respawn {
                    dimension: p.dimension.to_string(),
                    game_mode: GameMode::from_wire(p.gamemode),

                    keep: 0,
                }
            }
            cb::id::POSITION => {
                let p: cb::Position = raw.decode_as()?;
                Inbound::Teleport(Teleport {
                    x: p.x,
                    y: p.y,
                    z: p.z,
                    yaw: p.yaw,
                    pitch: p.pitch,
                    flags: p.flags,
                    teleport_id: Some(p.teleport_id),
                    relative: u32::from(p.flags as u8),
                    delta: Vec3::default(),
                })
            }
            cb::id::HELD_ITEM_SLOT => Inbound::HeldSlot(raw.decode_as::<cb::HeldItemSlot>()?.slot),
            cb::id::GAME_STATE_CHANGE => {
                let p: cb::GameStateChange = raw.decode_as()?;
                Inbound::GameState {
                    reason: p.reason.0,
                    value: p.game_mode,
                }
            }
            cb::id::OPEN_WINDOW => {
                Inbound::OpenWindow(i32::from(raw.decode_as::<cb::OpenWindow>()?.window_id))
            }
            cb::id::CLOSE_WINDOW => {
                Inbound::CloseWindow(i32::from(raw.decode_as::<cb::CloseWindow>()?.window_id))
            }
            cb::id::TRANSACTION => {
                let p: cb::Transaction = raw.decode_as()?;
                Inbound::Transaction {
                    window_id: p.window_id,
                    action: p.action,
                    accepted: p.accepted,
                }
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
                Inbound::Disconnect(raw.decode_as::<cb::KickDisconnect>()?.reason)
            }
            cb::id::ENTITY_VELOCITY => {
                let p: cb::EntityVelocity = raw.decode_as()?;
                Inbound::Velocity {
                    entity_id: p.entity_id,
                    motion: Vec3::new(
                        f64::from(p.velocity.x) / 8000.0,
                        f64::from(p.velocity.y) / 8000.0,
                        f64::from(p.velocity.z) / 8000.0,
                    ),
                }
            }
            cb::id::EXPLOSION => {
                let p: cb::Explosion = raw.decode_as()?;
                Inbound::ExplosionPush(Vec3::new(
                    f64::from(p.player_motion_x),
                    f64::from(p.player_motion_y),
                    f64::from(p.player_motion_z),
                ))
            }
            cb::id::ENTITY_UPDATE_ATTRIBUTES => {
                let p: cb::EntityUpdateAttributes = raw.decode_as()?;
                let Some(speed) = p.properties.iter().find(|a| {
                    a.key.ends_with("generic.movementSpeed")
                        || a.key.ends_with("generic.movement_speed")
                }) else {
                    return Ok(None);
                };
                Inbound::MovementSpeed {
                    entity_id: p.entity_id,
                    base: speed.value,
                    modifiers: speed
                        .modifiers
                        .iter()
                        .map(|m| (m.uuid, m.amount, m.operation))
                        .collect(),
                }
            }
            cb::id::ENTITY_EFFECT => {
                let p: cb::EntityEffect = raw.decode_as()?;
                Inbound::Effect {
                    entity_id: p.entity_id,
                    id: p.effect_id,
                    amplifier: p.amplifier,
                }
            }
            cb::id::REMOVE_ENTITY_EFFECT => {
                let p: cb::RemoveEntityEffect = raw.decode_as()?;
                Inbound::EffectRemoved {
                    entity_id: p.entity_id,
                    id: p.effect_id,
                }
            }
            cb::id::ENTITY_STATUS => {
                let p: cb::EntityStatus = raw.decode_as()?;
                Inbound::EntityStatus {
                    entity_id: p.entity_id,
                    status: p.entity_status,
                }
            }
            cb::id::COMBAT_EVENT => {
                let p: cb::CombatEvent = raw.decode_as()?;
                match (p.event, p.player_id) {
                    (COMBAT_ENTITY_DEAD, Some(entity_id)) => Inbound::Died { entity_id },
                    _ => return Ok(None),
                }
            }
            _ => return Ok(None),
        }))
    }

    const INVENTORY: ghast_inventory::Version = ghast_inventory::Version::V1_12;

    fn inventory_packet(
        inv: &mut ghast_inventory::Inventory,
        raw: &RawPacket,
    ) -> Result<Option<ghast_inventory::Outgoing>, DecodeError> {
        // Only the window packets are decoded (decode_raw would link every packet's decoder).
        use cb::ClientboundPlayPacket as P;
        let p = match raw.id {
            cb::id::LOGIN => P::Login(raw.decode_as()?),
            cb::id::RESPAWN => P::Respawn(raw.decode_as()?),
            cb::id::OPEN_WINDOW => P::OpenWindow(raw.decode_as()?),
            cb::id::CLOSE_WINDOW => P::CloseWindow(raw.decode_as()?),
            cb::id::WINDOW_ITEMS => P::WindowItems(raw.decode_as()?),
            cb::id::SET_SLOT => P::SetSlot(raw.decode_as()?),
            cb::id::TRANSACTION => P::Transaction(raw.decode_as()?),
            cb::id::HELD_ITEM_SLOT => P::HeldItemSlot(raw.decode_as()?),
            cb::id::CRAFT_PROGRESS_BAR => P::CraftProgressBar(raw.decode_as()?),
            _ => return Ok(None),
        };
        Ok(inv.apply_v1_12_2(&p))
    }

    fn write_play(enc: &mut FrameEncoder, out: &Outbound, (): &mut ()) -> Result<(), EncodeError> {
        match out {
            Outbound::Raw(p) => enc.write_raw(p),
            Outbound::KeepAlive(id) => enc.write_packet(&sb::KeepAlive { keep_alive_id: *id }),
            Outbound::TeleportConfirm(teleport_id) => enc.write_packet(&sb::TeleportConfirm {
                teleport_id: *teleport_id,
            }),
            Outbound::Move(m) => match *m {
                MovePacket::Flying { on_ground, .. } => enc.write_packet(&sb::Flying { on_ground }),
                MovePacket::Position { pos, on_ground, .. } => enc.write_packet(&sb::Position {
                    x: pos.x,
                    y: pos.y,
                    z: pos.z,
                    on_ground,
                }),
                MovePacket::Look {
                    yaw,
                    pitch,
                    on_ground,
                    ..
                } => enc.write_packet(&sb::Look {
                    yaw,
                    pitch,
                    on_ground,
                }),
                MovePacket::PositionLook {
                    pos,
                    yaw,
                    pitch,
                    on_ground,
                    ..
                } => enc.write_packet(&sb::PositionLook {
                    x: pos.x,
                    y: pos.y,
                    z: pos.z,
                    yaw,
                    pitch,
                    on_ground,
                }),
            },
            Outbound::EntityAction { entity_id, action } => {
                let action_id = match action {
                    EntityAction::StartSneaking => sb::EntityActionActionId::START_SNEAKING,
                    EntityAction::StopSneaking => sb::EntityActionActionId::STOP_SNEAKING,
                    EntityAction::StartSprinting => sb::EntityActionActionId::START_SPRINTING,
                    EntityAction::StopSprinting => sb::EntityActionActionId::STOP_SPRINTING,
                    // Only the 26.1 elytra start sends it.
                    EntityAction::StartFallFlying => return Ok(()),
                };
                enc.write_packet(&sb::EntityAction {
                    entity_id: *entity_id,
                    action_id,
                    jump_boost: 0,
                })
            }
            Outbound::Chat(text) => enc.write_packet(&sb::Chat {
                message: ByteString::from(truncate_utf16(text, Self::CHAT_LIMIT)),
            }),
            Outbound::Respawn => enc.write_packet(&sb::ClientCommand {
                action_id: sb::ClientCommandActionId::PERFORM_RESPAWN,
            }),
            Outbound::Transaction {
                window_id,
                action,
                accepted,
            } => enc.write_packet(&sb::Transaction {
                window_id: *window_id,
                action: *action,
                accepted: *accepted,
            }),
            Outbound::Abilities {
                flags,
                flying_speed,
                walking_speed,
            } => enc.write_packet(&sb::Abilities {
                flags: *flags,
                flying_speed: *flying_speed,
                walking_speed: *walking_speed,
            }),
            // Only the window packets' encoders (write_set would link every packet's).
            Outbound::Inventory(o) => match o.to_v1_12_2() {
                Some(sb::ServerboundPlayPacket::WindowClick(p)) => enc.write_packet(&p),
                Some(sb::ServerboundPlayPacket::Transaction(p)) => enc.write_packet(&p),
                Some(sb::ServerboundPlayPacket::CloseWindow(p)) => enc.write_packet(&p),
                _ => Ok(()),
            },
            Outbound::UseEntity {
                target,
                action,
                off_hand,
                ..
            } => {
                let (mouse, at) = match action {
                    crate::common::EntityUse::Interact => (0, None),
                    crate::common::EntityUse::Attack => (1, None),
                    crate::common::EntityUse::InteractAt(v) => (2, Some(*v)),
                };
                enc.write_packet(&sb::UseEntity {
                    target: *target,
                    mouse,
                    x: at.map(|v| v.x as f32),
                    y: at.map(|v| v.y as f32),
                    z: at.map(|v| v.z as f32),
                    hand: match action {
                        crate::common::EntityUse::Attack => None,
                        _ => Some(i32::from(*off_hand)),
                    },
                })
            }
            Outbound::HeldSlot(slot) => enc.write_packet(&sb::HeldItemSlot { slot_id: *slot }),
            Outbound::Swing => enc.write_packet(&sb::ArmAnimation { hand: 0 }),
            Outbound::Dig { status, pos, face } => enc.write_packet(&sb::BlockDig {
                status: status.id(),
                location: *pos,
                face: face.id(),
            }),
            Outbound::Place {
                pos,
                face,
                cursor,
                off_hand,
                ..
            } => enc.write_packet(&sb::BlockPlace {
                hand: i32::from(*off_hand),
                location: *pos,
                direction: i32::from(face.id()),
                cursor_x: cursor.x as f32,
                cursor_y: cursor.y as f32,
                cursor_z: cursor.z as f32,
            }),
            Outbound::UseItem { off_hand, .. } => enc.write_packet(&sb::UseItem {
                hand: i32::from(*off_hand),
            }),
            Outbound::SwingOffHand => enc.write_packet(&sb::ArmAnimation { hand: 1 }),
            Outbound::Settings(s) => enc.write_packet(&sb::Settings {
                // 1.11+ locales are lower case (`en_us`).
                locale: ByteString::from(s.locale.to_ascii_lowercase()),
                view_distance: s.view_distance,
                chat_flags: i32::from(s.chat_mode),
                chat_colors: s.chat_colors,
                skin_parts: s.skin_parts,
                main_hand: s.main_hand,
            }),
            Outbound::Brand(brand) => {
                let mut data = BytesMut::with_capacity(brand.len() + 1);
                data.put_string(brand)?;
                enc.write_packet(&sb::CustomPayload {
                    // Namespaced channels arrived with 1.13.
                    channel: ByteString::from_static("MC|Brand"),
                    data: data.freeze(),
                })
            }
            // Packets of newer versions: never produced for this one.
            Outbound::PlayerInput(_)
            | Outbound::TickEnd
            | Outbound::PlayerLoaded
            | Outbound::ChatSessionUpdate
            | Outbound::ChunkBatchReceived(_)
            | Outbound::Pong(_)
            | Outbound::ResourcePackStatus { .. }
            | Outbound::CookieResponse { .. }
            | Outbound::LookAnswer { .. }
            | Outbound::VehicleMove { .. }
            | Outbound::SteerBoat { .. }
            | Outbound::CustomClickAction { .. }
            | Outbound::DialogCommand(_) => Ok(()),
        }
    }

    fn apply_teleport(pose: &mut Pose, t: &Teleport) {
        let (yaw, pitch) = teleport_target(pose, t);
        // Entity.updatePositionAndAngles (1.9+): x / z clamped to ±3e7, pitch clamped to ±90,
        // both angles `% 360`.
        pose.pos.x = pose.pos.x.clamp(-3.0e7, 3.0e7);
        pose.pos.z = pose.pos.z.clamp(-3.0e7, 3.0e7);
        pose.yaw = yaw % 360.0;
        pose.pitch = pitch.clamp(-90.0, 90.0) % 360.0;
    }
}

impl super::HasSim for V1_12_2 {
    type Sim = crate::sim::Sim12;
    fn new_sim(opts: &crate::sim::SimOptions) -> Self::Sim {
        crate::sim::Sim12::with(opts)
    }
}
