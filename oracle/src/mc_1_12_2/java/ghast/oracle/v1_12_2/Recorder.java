package ghast.oracle.v1_12_2;

import ghast.oracle.Json;
import ghast.oracle.Scenario;
import net.minecraft.entity.Entity;
import net.minecraft.entity.LivingEntity;
import net.minecraft.entity.attribute.EntityAttributes;
import net.minecraft.entity.player.ClientPlayerEntity;
import net.minecraft.entity.player.PlayerAbilities;
import net.minecraft.entity.player.PlayerEntity;
import net.minecraft.network.Packet;
import net.minecraft.network.packet.c2s.play.ClientCommandC2SPacket;
import net.minecraft.network.packet.c2s.play.PlayerActionC2SPacket;
import net.minecraft.network.packet.c2s.play.PlayerInteractBlockC2SPacket;
import net.minecraft.network.packet.c2s.play.PlayerMoveC2SPacket;
import net.minecraft.network.packet.c2s.play.UpdatePlayerAbilitiesC2SPacket;
import net.minecraft.network.packet.c2s.play.TeleportConfirmC2SPacket;
import net.minecraft.network.packet.c2s.play.SwingHandC2SPacket;
import net.minecraft.network.packet.c2s.play.HandSwingC2SPacket;
import net.minecraft.network.packet.c2s.play.UpdateSelectedSlotC2SPacket;
import net.minecraft.util.math.Box;


/** Serialises scenario headers, player state and captured packets (schema: oracle/README.md). */
final class Recorder {
    private Recorder() {}

    static String header(Scenario sc, Harness.Run run, String version) {
        Json j = new Json().beginObject();
        j.field("type", "scenario");
        j.field("format", 1);
        j.field("version", version);
        j.field("name", sc.name);
        j.field("description", sc.description);
        j.field("outside", sc.outside.name().toLowerCase());
        j.key("loaded_chunks").beginArray();
        for (int[] r : run.loadedRects) {
            j.beginArray().value(r[0]).value(r[1]).value(r[2]).value(r[3]).endArray();
        }
        j.endArray();
        j.key("fills").beginArray();
        for (Scenario.Fill f : sc.fills) {
            j.beginArray().value(f.x0()).value(f.y0()).value(f.z0()).value(f.x1()).value(f.y1()).value(f.z1())
                    .value(f.id()).value(f.meta()).endArray();
        }
        j.endArray();
        j.key("setup").beginObject();
        j.field("game_mode", sc.gameMode.name().toLowerCase());
        j.key("pos").beginArray().f64(sc.x).f64(sc.y).f64(sc.z).endArray();
        j.fieldF32("yaw", sc.yaw);
        j.fieldF32("pitch", sc.pitch);
        j.key("vel").beginArray().f64(sc.vx).f64(sc.vy).f64(sc.vz).endArray();
        j.field("on_ground", sc.onGround);
        j.fieldF32("fall_distance", sc.fallDistance);
        j.field("sprinting", sc.sprinting);
        j.field("flying", sc.flying);
        if (sc.allowFlying != null) {
            j.field("allow_flying", sc.allowFlying);
        }
        j.fieldF32("fly_speed", sc.flySpeed);
        j.fieldF32("walk_speed", sc.walkSpeed);
        j.fieldF64("movement_speed_base", sc.movementSpeedBase);
        if (sc.bb != null) {
            j.key("bb").beginArray();
            for (double d : sc.bb) {
                j.f64(d);
            }
            j.endArray();
        }
        if (sc.pose != null) {
            j.field("pose", sc.pose);
        }
        if (!sc.speedModifiers.isEmpty()) {
            j.key("speed_modifiers").beginArray();
            for (double[] m : sc.speedModifiers) {
                j.beginArray().f64(m[0]).value((long) m[1]).endArray();
            }
            j.endArray();
        }
        if (sc.depthStrider > 0) {
            j.field("depth_strider", sc.depthStrider);
        }
        j.field("food_level", sc.foodLevel);
        j.key("effects").beginArray();
        sc.effects.forEach((id, amp) -> j.beginArray().value(id).value(amp).endArray());
        j.endArray();
        j.endObject();
        j.field("ticks", sc.ticks.size());
        return j.endObject().toString();
    }

    /** Tick record: {@code tick} 0 is the state right after setup (no input yet). */
    static String tick(Harness.Run run, Scenario.Tick input) {
        Json j = new Json().beginObject();
        j.field("type", "tick");
        j.field("t", run.tick);
        if (input != null) {
            j.key("input").beginObject();
            j.field("keys", Scenario.keyNames(input.keys()));
            j.fieldF32("yaw", input.yaw());
            j.fieldF32("pitch", input.pitch());
            if (!input.events().isEmpty()) {
                j.key("events").beginArray();
                for (Scenario.Event e : input.events()) {
                    event(j, e);
                }
                j.endArray();
            }
            j.endObject();
        }
        j.key("state");
        state(j, run.player, run.sc.usesItem());
        j.key("packets").beginArray();
        for (Packet<?> p : run.net.sent) {
            packet(j, p);
        }
        j.endArray();
        run.net.sent.clear();
        return j.endObject().toString();
    }

    private static void event(Json j, Scenario.Event e) {
        j.beginObject();
        if (e instanceof Scenario.Velocity v) {
            j.field("type", "velocity").field("x", v.x()).field("y", v.y()).field("z", v.z());
        } else if (e instanceof Scenario.Teleport t) {
            j.field("type", "teleport");
            j.key("pos").beginArray().f64(t.x()).f64(t.y()).f64(t.z()).endArray();
            j.fieldF32("yaw", t.yaw()).fieldF32("pitch", t.pitch());
            if (t.flags() != 0) {
                j.field("flags", t.flags());
            }
        } else if (e instanceof Scenario.SpeedAttribute sa) {
            j.field("type", "speed_attribute").fieldF64("base", sa.base());
            j.key("modifiers").beginArray();
            for (Scenario.Modifier m : sa.modifiers()) {
                j.beginArray().value(m.uuid().toString()).f64(m.amount()).value(m.operation()).endArray();
            }
            j.endArray();
        } else if (e instanceof Scenario.Effects fx) {
            j.field("type", "effects");
            j.key("effects").beginArray();
            for (int[] f : fx.effects()) {
                j.beginArray().value(f[0]).value(f[1]).endArray();
            }
            j.endArray();
        } else if (e instanceof Scenario.Abilities a) {
            j.field("type", "abilities").field("invulnerable", a.invulnerable()).field("flying", a.flying())
                    .field("allow_flying", a.allowFlying()).field("creative", a.creative())
                    .fieldF32("fly_speed", a.flySpeed()).fieldF32("walk_speed", a.walkSpeed());
        }
        j.endObject();
    }

    static void state(Json j, ClientPlayerEntity p, boolean withItemUse) {
        j.beginObject();
        j.key("pos").beginArray().f64(p.x).f64(p.y).f64(p.z).endArray();
        Box bb = p.getBoundingBox();
        j.key("bb").beginArray().f64(bb.minX).f64(bb.minY).f64(bb.minZ).f64(bb.maxX).f64(bb.maxY).f64(bb.maxZ).endArray();
        j.key("vel").beginArray().f64(p.velocityX).f64(p.velocityY).f64(p.velocityZ).endArray();
        j.fieldF32("yaw", p.yaw);
        j.fieldF32("pitch", p.pitch);
        j.field("on_ground", p.onGround);
        j.field("horizontal_collision", p.horizontalCollision);
        j.field("vertical_collision", p.verticalCollision);
        j.field("colliding", p.colliding);
        j.field("velocity_dirty", p.velocityDirty);
        j.fieldF32("fall_distance", p.fallDistance);
        j.field("in_web", Reflect.getBoolean(Entity.class, p, "inWeb"));
        j.field("touching_water", Reflect.getBoolean(Entity.class, p, "touchingWater"));
        j.field("touching_lava", p.isTouchingLava());
        j.field("climbing", p.isClimbing());
        j.field("sprinting", p.isSprinting());
        j.field("sneaking", p.isSneaking());
        j.field("outside_world_border", p.isOutsideWorldBorder());
        j.field("no_clip", p.noClip);
        j.fieldF32("step_height", p.stepHeight);
        // 1.9+: the player's size follows the pose (sneaking 0.6 x 1.65, standing 0.6 x 1.8).
        j.fieldF32("width", p.width);
        j.fieldF32("height", p.height);

        PlayerAbilities ab = p.abilities;
        j.field("flying", ab.flying);
        j.field("allow_flying", ab.allowFlying);
        j.field("creative", ab.creativeMode);
        j.field("invulnerable", ab.invulnerable);
        j.fieldF32("fly_speed", ab.getFlySpeed());
        j.fieldF32("walk_speed", ab.getWalkSpeed());

        // LivingEntity
        j.field("jump_cooldown", Reflect.getInt(LivingEntity.class, p, "jumpingCooldown"));
        j.field("jumping", Reflect.getBoolean(LivingEntity.class, p, "jumping"));
        // 1.12 Legacy Yarn: field_16513 is the forward input, `forwardSpeed` the (always 0) vertical one.
        j.fieldF32("forward_speed", p.field_16513);
        j.fieldF32("sideways_speed", p.sidewaysSpeed);
        j.fieldF32("upward_speed", p.forwardSpeed);
        j.fieldF32("flying_speed", p.flyingSpeed);
        j.fieldF32("movement_speed", Reflect.getFloat(LivingEntity.class, p, "movementSpeed"));
        j.fieldF64("movement_speed_attr", p.initializeAttribute(EntityAttributes.GENERIC_MOVEMENT_SPEED).getValue());

        // PlayerEntity / ClientPlayerEntity timers
        j.field("fly_toggle_timer", Reflect.getInt(PlayerEntity.class, p, "abilityResyncCountdown"));
        j.field("sprint_toggle_timer", Reflect.getInt(ClientPlayerEntity.class, p, "field_1763"));
        // 1.9+: ticksSinceSprintingChanged counts UP (reset to 0 by every setSprinting); it no longer
        // ends a sprint. The key keeps its 1.8.9 name.
        j.field("sprinting_ticks_left", p.ticksSinceSprintingChanged);
        j.field("food_level", p.getHungerManager().getFoodLevel());
        if (withItemUse) {
            // Only in scenarios that use the use key, so files without item use keep their old schema.
            j.field("using_item", p.method_13061());
        }
        j.field("ticks_alive", p.ticksAlive);

        // Input after this tick's KeyboardInput.tick() (and the using-item slowdown).
        j.key("input").beginObject();
        j.fieldF32("forward", p.input.movementForward);
        j.fieldF32("sideways", p.input.movementSideways);
        j.field("jump", p.input.jumping);
        j.field("sneak", p.input.sneaking);
        j.endObject();

        // What the server was last told (sendMovementPackets bookkeeping).
        j.key("sent").beginObject();
        j.key("pos").beginArray()
                .f64(Reflect.getDouble(ClientPlayerEntity.class, p, "lastX"))
                .f64(Reflect.getDouble(ClientPlayerEntity.class, p, "lastBaseY"))
                .f64(Reflect.getDouble(ClientPlayerEntity.class, p, "lastZ")).endArray();
        j.fieldF32("yaw", Reflect.getFloat(ClientPlayerEntity.class, p, "lastYaw"));
        j.fieldF32("pitch", Reflect.getFloat(ClientPlayerEntity.class, p, "lastPitch"));
        j.field("sprinting", Reflect.getBoolean(ClientPlayerEntity.class, p, "lastSprinting"));
        j.field("sneaking", Reflect.getBoolean(ClientPlayerEntity.class, p, "lastIsHoldingSneakKey"));
        j.field("ticks_since_pos", Reflect.getInt(ClientPlayerEntity.class, p, "ticksSinceLastPositionPacketSent"));
        // 1.9+: the last on_ground sent; a bare on_ground packet goes out only when it changes.
        j.field("on_ground", Reflect.getBoolean(ClientPlayerEntity.class, p, "field_13457"));
        j.endObject();
        j.endObject();
    }

    /** Packet names follow minecraft-data's 1.12.2 play.toServer names. */
    static void packet(Json j, Packet<?> pk) {
        j.beginObject();
        if (pk instanceof PlayerMoveC2SPacket m) {
            boolean pos = Reflect.getBoolean(PlayerMoveC2SPacket.class, m, "changePosition");
            boolean look = Reflect.getBoolean(PlayerMoveC2SPacket.class, m, "changeLook");
            String name = pos ? (look ? "position_look" : "position") : (look ? "look" : "flying");
            j.field("name", name);
            if (pos) {
                j.key("pos").beginArray().f64(m.method_12687(0)).f64(m.method_12689(0)).f64(m.method_12691(0)).endArray();
            }
            if (look) {
                j.fieldF32("yaw", m.method_12688(0)).fieldF32("pitch", m.method_12690(0));
            }
            j.field("on_ground", m.isOnGround());
        } else if (pk instanceof ClientCommandC2SPacket c) {
            j.field("name", "entity_action");
            j.field("action", c.getMode().ordinal());
            j.field("action_name", c.getMode().name().toLowerCase());
        } else if (pk instanceof UpdatePlayerAbilitiesC2SPacket a) {
            j.field("name", "abilities");
            j.field("invulnerable", a.isInvulnerable()).field("flying", a.isFlying())
                    .field("allow_flying", a.isFlyingAllowed()).field("creative", a.isCreativeMode());
            j.fieldF32("fly_speed", Reflect.getFloat(UpdatePlayerAbilitiesC2SPacket.class, a, "flySpeed"));
            j.fieldF32("walk_speed", Reflect.getFloat(UpdatePlayerAbilitiesC2SPacket.class, a, "walkSpeed"));
        } else if (pk instanceof TeleportConfirmC2SPacket t) {
            j.field("name", "teleport_confirm");
            j.field("teleport_id", t.getTeleportId());
        } else if (pk instanceof SwingHandC2SPacket u) {
            // The use-item packet (minecraft-data `use_item`); Legacy Yarn calls the class SwingHandC2SPacket.
            j.field("name", "use_item");
            j.field("hand", u.getHand().ordinal());
        } else if (pk instanceof HandSwingC2SPacket s) {
            j.field("name", "arm_animation");
            j.field("hand", s.method_12707().ordinal());
        } else if (pk instanceof UpdateSelectedSlotC2SPacket h) {
            j.field("name", "held_item_slot");
            j.field("slot", h.getSelectedSlot());
        } else if (pk instanceof PlayerInteractBlockC2SPacket b) {
            j.field("name", "block_place");
            j.key("location").beginArray().value(b.getPos().getX()).value(b.getPos().getY()).value(b.getPos().getZ()).endArray();
            j.field("direction", b.method_12708().getId());
            j.field("hand", b.method_12709().ordinal());
        } else if (pk instanceof PlayerActionC2SPacket d) {
            j.field("name", "block_dig");
            j.field("status", d.getAction().ordinal());
            j.field("status_name", d.getAction().name().toLowerCase());
        } else {
            j.field("name", "other");
            j.field("class", pk.getClass().getName());
        }
        j.endObject();
    }
}
