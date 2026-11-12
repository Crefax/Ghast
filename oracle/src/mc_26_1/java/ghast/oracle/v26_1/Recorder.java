package ghast.oracle.v26_1;

import ghast.oracle.Json;
import ghast.oracle.Scenario;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ServerboundAcceptTeleportationPacket;
import net.minecraft.network.protocol.game.ServerboundClientTickEndPacket;
import net.minecraft.network.protocol.game.ServerboundMovePlayerPacket;
import net.minecraft.network.protocol.game.ServerboundPlayerAbilitiesPacket;
import net.minecraft.network.protocol.game.ServerboundPlayerActionPacket;
import net.minecraft.network.protocol.game.ServerboundPlayerCommandPacket;
import net.minecraft.network.protocol.game.ServerboundPlayerInputPacket;
import net.minecraft.network.protocol.game.ServerboundPlayerLoadedPacket;
import net.minecraft.network.protocol.game.ServerboundSetCarriedItemPacket;
import net.minecraft.network.protocol.game.ServerboundSwingPacket;
import net.minecraft.network.protocol.game.ServerboundUseItemPacket;
import net.minecraft.tags.FluidTags;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.ai.attributes.Attribute;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec2;
import net.minecraft.world.phys.Vec3;

import java.util.Map;
import java.util.Optional;

/** Serialises 26.1 scenario headers, player state and captured packets (schema: oracle/README.md, "26.1"). */
final class Recorder {
    private Recorder() {}

    /** Attributes recorded every tick (the movement-relevant ones; values after modifiers). */
    @SuppressWarnings("unchecked")
    static final Map.Entry<String, Holder<Attribute>>[] ATTRIBUTES = new Map.Entry[] {
            Map.entry("movement_speed", Attributes.MOVEMENT_SPEED), Map.entry("step_height", Attributes.STEP_HEIGHT),
            Map.entry("gravity", Attributes.GRAVITY), Map.entry("jump_strength", Attributes.JUMP_STRENGTH),
            Map.entry("sneaking_speed", Attributes.SNEAKING_SPEED), Map.entry("movement_efficiency", Attributes.MOVEMENT_EFFICIENCY),
            Map.entry("water_movement_efficiency", Attributes.WATER_MOVEMENT_EFFICIENCY), Map.entry("scale", Attributes.SCALE),
            Map.entry("safe_fall_distance", Attributes.SAFE_FALL_DISTANCE),
    };

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
        // [x0, y0, z0, x1, y1, z1, state] with the 26.1 global block state id.
        j.key("fills").beginArray();
        for (Scenario.Fill f : sc.fills) {
            j.beginArray().value(f.x0()).value(f.y0()).value(f.z0()).value(f.x1()).value(f.y1()).value(f.z1())
                    .value(f.id()).endArray();
        }
        j.endArray();
        j.key("states").beginObject();
        for (Map.Entry<Integer, String> e : run.statesUsed.entrySet()) {
            j.field(Integer.toString(e.getKey()), e.getValue());
        }
        j.endObject();
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
        j.key("speed_modifiers").beginArray();
        for (double[] m : sc.speedModifiers) {
            j.beginArray().f64(m[0]).value((long) m[1]).endArray();
        }
        j.endArray();
        j.key("attribute_bases").beginObject();
        sc.attributeBases.forEach((k, v) -> j.fieldF64(k, v));
        j.endObject();
        j.key("attribute_modifiers").beginArray();
        for (Scenario.AttributeModifierSpec m : sc.attributeModifiers) {
            j.beginArray().value(m.attribute()).value(m.id()).f64(m.amount()).value(m.operation()).endArray();
        }
        j.endArray();
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
        j.field("food_level", sc.foodLevel);
        // Effects by 26.1 registry id (network id), with the name for readers.
        j.key("effects").beginArray();
        sc.effects.forEach((id, amp) -> j.beginArray().value(id).value(amp)
                .value(net.minecraft.core.registries.BuiltInRegistries.MOB_EFFECT.byId(id) == null ? "?"
                        : net.minecraft.core.registries.BuiltInRegistries.MOB_EFFECT.getKey(
                                net.minecraft.core.registries.BuiltInRegistries.MOB_EFFECT.byId(id)).toString()).endArray());
        j.endArray();
        j.field("auto_jump", sc.autoJump);
        j.field("soul_speed", sc.soulSpeed);
        j.field("depth_strider", sc.depthStrider);
        // 26.x setup extras (only when set, so older files keep their bytes).
        if (sc.leatherBoots) {
            j.field("leather_boots", true);
        }
        if (sc.useSpeed != null || sc.useCanSprint != null) {
            j.key("use_effects").beginObject().field("can_sprint", sc.useCanSprint != null && sc.useCanSprint)
                    .fieldF32("speed_multiplier", sc.useSpeed != null ? sc.useSpeed : 0.2F).endObject();
        }
        if (sc.sprintWindow != null) {
            j.field("sprint_window", sc.sprintWindow);
        }
        if (sc.delayedLoad) {
            j.field("delayed_load", true);
        }
        if (sc.health != null) {
            j.fieldF32("health", sc.health);
        }
        Ext.header(j, sc);
        j.endObject();
        j.field("ticks", sc.ticks.size());
        return j.endObject().toString();
    }

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
        if (run.sc.delayedLoad) {
            j.field("client_loaded", Harness.listener.hasClientLoaded());
        }
        j.key("state");
        state(j, run.player);
        Ext.tickRecord(j, run);
        j.key("packets").beginArray();
        for (Packet<?> p : Harness.connection.sent) {
            packet(j, p);
        }
        j.endArray();
        Harness.connection.sent.clear();
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
            if (t.hasDelta()) {
                j.key("delta").beginArray().f64(t.dx()).f64(t.dy()).f64(t.dz()).endArray();
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
        } else if (e instanceof Scenario.Attribute at) {
            j.field("type", "attribute").field("attribute", at.attribute()).fieldF64("base", at.base());
            j.key("modifiers").beginArray();
            for (Scenario.AttributeModifierSpec m : at.modifiers()) {
                j.beginArray().value(m.id()).f64(m.amount()).value(m.operation()).endArray();
            }
            j.endArray();
        } else if (e instanceof Scenario.Abilities a) {
            j.field("type", "abilities").field("invulnerable", a.invulnerable()).field("flying", a.flying())
                    .field("allow_flying", a.allowFlying()).field("creative", a.creative())
                    .fieldF32("fly_speed", a.flySpeed()).fieldF32("walk_speed", a.walkSpeed());
        } else if (e instanceof Scenario.LoadStart) {
            j.field("type", "level_chunks_load_start");
        } else if (e instanceof Scenario.SectionCompiled c) {
            j.field("type", "section_compiled").field("compiled", c.compiled());
        } else if (e instanceof Scenario.LoadTimeout) {
            j.field("type", "load_timeout");
        } else if (e instanceof Scenario.VersionEvent ve) {
            Ext.event(j, ve);
        }
        j.endObject();
    }

    static void state(Json j, LocalPlayer p) {
        j.beginObject();
        j.key("pos").beginArray().f64(p.getX()).f64(p.getY()).f64(p.getZ()).endArray();
        AABB bb = p.getBoundingBox();
        j.key("bb").beginArray().f64(bb.minX).f64(bb.minY).f64(bb.minZ).f64(bb.maxX).f64(bb.maxY).f64(bb.maxZ).endArray();
        Vec3 v = p.getDeltaMovement();
        j.key("vel").beginArray().f64(v.x).f64(v.y).f64(v.z).endArray();
        j.fieldF32("yaw", p.getYRot());
        j.fieldF32("pitch", p.getXRot());
        j.field("on_ground", p.onGround());
        j.field("horizontal_collision", p.horizontalCollision);
        j.field("minor_horizontal_collision", p.minorHorizontalCollision);
        j.field("vertical_collision", p.verticalCollision);
        j.field("vertical_collision_below", p.verticalCollisionBelow);
        j.field("velocity_modified", p.hurtMarked);
        j.fieldF64("fall_distance", p.fallDistance);
        Vec3 mm = (Vec3) Reflect.get(Entity.class, p, "stuckSpeedMultiplier");
        j.key("movement_multiplier").beginArray().f64(mm.x).f64(mm.y).f64(mm.z).endArray();
        @SuppressWarnings("unchecked")
        Optional<BlockPos> support = (Optional<BlockPos>) Reflect.get(Entity.class, p, "mainSupportingBlockPos");
        j.key("supporting_block");
        if (support.isPresent()) {
            BlockPos s = support.get();
            j.beginArray().value(s.getX()).value(s.getY()).value(s.getZ()).endArray();
        } else {
            j.raw("null");
        }
        j.field("on_ground_no_blocks", Reflect.getBoolean(Entity.class, p, "onGroundNoBlocks"));
        j.field("pose", p.getPose().name().toLowerCase());
        j.fieldF32("width", p.getBbWidth());
        j.fieldF32("height", p.getBbHeight());
        j.fieldF32("eye_height", p.getEyeHeight());
        j.field("touching_water", p.isInWater());
        j.field("submerged_in_water", p.isUnderWater());
        j.field("submerged_fluid", p.isEyeInFluid(FluidTags.WATER) ? "water" : p.isEyeInFluid(FluidTags.LAVA) ? "lava" : null);
        j.fieldF64("water_height", p.getFluidHeight(FluidTags.WATER));
        j.fieldF64("lava_height", p.getFluidHeight(FluidTags.LAVA));
        j.field("in_lava", p.isInLava());
        j.field("in_powder_snow", p.isInPowderSnow);
        j.field("was_in_powder_snow", p.wasInPowderSnow);
        j.field("first_update", Reflect.getBoolean(Entity.class, p, "firstTick"));
        j.field("climbing", p.onClimbable());
        j.field("swimming", p.isSwimming());
        j.field("sprinting", p.isSprinting());
        j.field("sneaking", p.isShiftKeyDown());
        j.field("in_sneaking_pose", p.isCrouching());
        j.field("visually_crawling", p.isVisuallyCrawling());
        j.field("fall_flying", p.isFallFlying());
        j.field("using_item", p.isUsingItem());
        j.field("no_clip", p.noPhysics);
        j.fieldF32("step_height", p.maxUpStep());

        Abilities ab = p.getAbilities();
        j.field("flying", ab.flying);
        j.field("allow_flying", ab.mayfly);
        j.field("creative", ab.instabuild);
        j.field("invulnerable", ab.invulnerable);
        j.fieldF32("fly_speed", ab.getFlyingSpeed());
        j.fieldF32("walk_speed", ab.getWalkingSpeed());

        j.field("jump_cooldown", Reflect.getInt(LivingEntity.class, p, "noJumpDelay"));
        j.field("jumping", Reflect.getBoolean(LivingEntity.class, p, "jumping"));
        j.fieldF32("forward_speed", p.zza);
        j.fieldF32("sideways_speed", p.xxa);
        j.fieldF32("upward_speed", p.yya);
        j.fieldF32("flying_speed", flyingSpeed(p));
        j.fieldF32("movement_speed", p.getSpeed());
        j.key("attributes").beginObject();
        for (Map.Entry<String, Holder<Attribute>> a : ATTRIBUTES) {
            j.fieldF64(a.getKey(), p.getAttributeValue(a.getValue()));
        }
        j.endObject();
        j.fieldF64("movement_speed_attr", p.getAttributeValue(Attributes.MOVEMENT_SPEED));
        Object lastPos = Reflect.get(LivingEntity.class, p, "lastPos");
        j.field("last_block_pos", lastPos == null ? null : lastPos.toString());

        j.field("fly_toggle_timer", Reflect.getInt(Player.class, p, "jumpTriggerTime"));
        j.field("sprint_toggle_timer", Reflect.getInt(LocalPlayer.class, p, "sprintTriggerTime"));
        j.field("auto_jump_enabled", p.isAutoJumpEnabled());
        j.field("ticks_to_next_autojump", Reflect.getInt(LocalPlayer.class, p, "autoJumpTime"));
        j.field("food_level", p.getFoodData().getFoodLevel());
        j.field("age", p.tickCount);

        j.key("input").beginObject();
        Vec2 mv = p.input.getMoveVector();
        j.key("move_vector").beginArray().f32(mv.x).f32(mv.y).endArray();
        input(j, "keys", p.input.keyPresses);
        j.endObject();

        j.key("sent").beginObject();
        j.key("pos").beginArray()
                .f64(Reflect.getDouble(LocalPlayer.class, p, "xLast"))
                .f64(Reflect.getDouble(LocalPlayer.class, p, "yLast"))
                .f64(Reflect.getDouble(LocalPlayer.class, p, "zLast")).endArray();
        j.fieldF32("yaw", Reflect.getFloat(LocalPlayer.class, p, "yRotLast"));
        j.fieldF32("pitch", Reflect.getFloat(LocalPlayer.class, p, "xRotLast"));
        j.field("on_ground", Reflect.getBoolean(LocalPlayer.class, p, "lastOnGround"));
        j.field("horizontal_collision", Reflect.getBoolean(LocalPlayer.class, p, "lastHorizontalCollision"));
        j.field("sprinting", Reflect.getBoolean(LocalPlayer.class, p, "wasSprinting"));
        j.field("ticks_since_pos", Reflect.getInt(LocalPlayer.class, p, "positionReminder"));
        input(j, "input", p.getLastSentInput());
        j.endObject();
        j.endObject();
    }

    private static float flyingSpeed(LocalPlayer p) {
        try {
            java.lang.reflect.Method m = Player.class.getDeclaredMethod("getFlyingSpeed");
            m.setAccessible(true);
            return (float) m.invoke(p);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
    }

    private static void input(Json j, String key, Input in) {
        j.key(key).beginObject().field("forward", in.forward()).field("backward", in.backward()).field("left", in.left())
                .field("right", in.right()).field("jump", in.jump()).field("shift", in.shift()).field("sprint", in.sprint())
                .endObject();
    }

    /** Packet names follow minecraft-data's 1.21.x play.toServer names. */
    static void packet(Json j, Packet<?> pk) {
        j.beginObject();
        if (pk instanceof ServerboundMovePlayerPacket m) {
            boolean pos = m.hasPosition(), look = m.hasRotation();
            j.field("name", pos ? (look ? "position_look" : "position") : (look ? "look" : "flying"));
            if (pos) {
                j.key("pos").beginArray().f64(m.getX(0)).f64(m.getY(0)).f64(m.getZ(0)).endArray();
            }
            if (look) {
                j.fieldF32("yaw", m.getYRot(0)).fieldF32("pitch", m.getXRot(0));
            }
            j.field("on_ground", m.isOnGround());
            j.field("horizontal_collision", m.horizontalCollision());
        } else if (pk instanceof ServerboundPlayerCommandPacket c) {
            j.field("name", "entity_action");
            j.field("action", c.getAction().ordinal());
            j.field("action_name", c.getAction().name().toLowerCase());
        } else if (pk instanceof ServerboundPlayerInputPacket in) {
            j.field("name", "player_input");
            input(j, "input", in.input());
        } else if (pk instanceof ServerboundClientTickEndPacket) {
            j.field("name", "tick_end");
        } else if (pk instanceof ServerboundPlayerLoadedPacket) {
            j.field("name", "player_loaded");
        } else if (pk instanceof ServerboundPlayerAbilitiesPacket a) {
            j.field("name", "abilities").field("flying", a.isFlying());
        } else if (pk instanceof ServerboundAcceptTeleportationPacket t) {
            j.field("name", "teleport_confirm").field("teleport_id", t.getId());
        } else if (pk instanceof ServerboundUseItemPacket u) {
            j.field("name", "use_item").field("hand", u.getHand().ordinal()).field("sequence", u.getSequence());
            j.fieldF32("yaw", u.getYRot()).fieldF32("pitch", u.getXRot());
        } else if (pk instanceof ServerboundPlayerActionPacket d) {
            j.field("name", "block_dig");
            j.field("status", d.getAction().ordinal());
            j.field("status_name", d.getAction().name().toLowerCase());
            j.field("sequence", d.getSequence());
        } else if (pk instanceof ServerboundSetCarriedItemPacket s) {
            j.field("name", "held_item_slot").field("slot", s.getSlot());
        } else if (pk instanceof ServerboundSwingPacket s) {
            j.field("name", "arm_animation").field("hand", s.getHand().ordinal());
        } else {
            j.field("name", "other");
            j.field("class", pk.getClass().getName());
        }
        j.endObject();
    }
}
