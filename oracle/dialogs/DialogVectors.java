// Dialog answer vectors (tests/golden/26.2/dialog_answers.txt) from the vanilla client classes.
//
// Needs the 26.2 jar and libraries the oracle setup downloads (`MC=26.2 oracle/run.sh` once) and a
// JDK 25. From the repository root:
//
//   CP=oracle/cache/26.2/client-official.jar:$(paste -sd: oracle/cache/26.2/classpath.txt)
//   javac -cp "$CP" -d /tmp/dlg oracle/dialogs/DialogVectors.java
//   java -cp "/tmp/dlg:$CP" DialogVectors < oracle/dialogs/cases.txt   # the vector lines
//
// The GUI controls cannot run headless, so their value getters are rebuilt here from the same
// vanilla helpers they call (StringTag.escapeWithoutQuotes, StringUtil.filterText, the slider's
// own value functions through reflection); everything else is the real code path: the dialog
// codec, Action.createAction and the packets' stream codecs.

import io.netty.buffer.ByteBuf;
import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.lang.reflect.Method;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.commands.Commands;
import net.minecraft.nbt.*;
import net.minecraft.network.chat.ClickEvent;
import net.minecraft.network.protocol.common.ServerboundCustomClickActionPacket;
import net.minecraft.network.protocol.game.ServerboundChatCommandPacket;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.dialog.*;
import net.minecraft.server.dialog.action.Action;
import net.minecraft.server.dialog.input.*;
import net.minecraft.util.Mth;
import net.minecraft.util.StringUtil;

/**
 * Dialog answer vectors from the vanilla classes: each case is a dialog (as the server's NBT), a
 * button (or Escape), the input values, and the serverbound packet body vanilla writes.
 * The GUI controls' value getters are reproduced with the same vanilla helpers they call.
 */
public class DialogVectors {
    static Method sliderString, sliderScaled;

    public static void main(String[] args) throws Exception {
        java.io.PrintStream out = System.out;
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        System.setOut(out);
        Class<?> slider = Class.forName("net.minecraft.client.gui.screens.dialog.input.InputControlHandlers$NumberRangeHandler$SliderImpl");
        sliderString = slider.getDeclaredMethod("sliderValueToString", NumberRangeInput.class, double.class);
        sliderString.setAccessible(true);
        sliderScaled = slider.getDeclaredMethod("scaledValue", NumberRangeInput.class, double.class);
        sliderScaled.setAccessible(true);
        Scanner in = new Scanner(System.in);
        while (in.hasNextLine()) {
            String line = in.nextLine().trim();
            if (line.isEmpty() || line.startsWith("//")) continue;
            // name | dialog snbt | pick (button N / exit / close) | values snbt
            String[] f = line.split(" \\| ");
            CompoundTag dtag = TagParser.parseCompoundFully(f[1]);
            Dialog d = Dialog.DIRECT_CODEC.parse(NbtOps.INSTANCE, dtag).getOrThrow();
            CompoundTag given = TagParser.parseCompoundFully(f[3]);
            Map<String, Action.ValueGetter> getters = new HashMap<>();
            for (Input input : d.common().inputs()) {
                getters.put(input.key(), getter(input, given.get(input.key())));
            }
            Optional<Action> action;
            String pick = f[2];
            if (pick.startsWith("button ")) {
                int i = Integer.parseInt(pick.substring(7));
                List<ActionButton> buttons = d instanceof SimpleDialog s ? s.mainActions() : ((MultiActionDialog) d).actions();
                action = buttons.get(i).action();
            } else if (pick.equals("exit")) {
                action = ((ButtonListDialog) d).exitAction().flatMap(ActionButton::action);
            } else {
                action = d.onCancel();
            }
            Optional<ClickEvent> ev = action.flatMap(a -> a.createAction(getters));
            String sent;
            if (ev.isEmpty()) {
                sent = "none";
            } else if (ev.get() instanceof ClickEvent.Custom c) {
                ByteBuf b = Unpooled.buffer();
                ServerboundCustomClickActionPacket.STREAM_CODEC.encode(b, new ServerboundCustomClickActionPacket(c.id(), c.payload()));
                sent = "custom_click_action " + ByteBufUtil.hexDump(b);
            } else if (ev.get() instanceof ClickEvent.RunCommand r) {
                ByteBuf b = Unpooled.buffer();
                ServerboundChatCommandPacket.STREAM_CODEC.encode(new net.minecraft.network.FriendlyByteBuf(b), new ServerboundChatCommandPacket(Commands.trimOptionalPrefix(r.command())));
                sent = "chat_command " + ByteBufUtil.hexDump(b);
            } else {
                sent = "client " + ev.get().action().getSerializedName();
            }
            ByteArrayOutputStream nbt = new ByteArrayOutputStream();
            NbtIo.writeAnyTag(dtag, new DataOutputStream(nbt));
            System.out.println(f[0] + " " + HexFormat.of().formatHex(nbt.toByteArray()) + " " + pick.replace(' ', '_') + " "
                + HexFormat.of().formatHex(writeAny(given)) + " " + sent);
        }
    }

    static byte[] writeAny(Tag t) throws Exception {
        ByteArrayOutputStream o = new ByteArrayOutputStream();
        NbtIo.writeAnyTag(t, new DataOutputStream(o));
        return o.toByteArray();
    }

    /** The value getter of the input's control, its value set as a player would. */
    static Action.ValueGetter getter(Input input, Tag value) throws Exception {
        InputControl c = input.control();
        if (c instanceof TextInput t) {
            String v;
            if (value == null) {
                v = t.initial().length() > t.maxLength() ? t.initial().substring(0, t.maxLength()) : t.initial();
            } else {
                // Select all, then type: EditBox.insertText.
                String text = StringUtil.filterText(value.asString().orElseThrow(), t.multiline().isPresent());
                int max = t.maxLength();
                if (max < text.length()) {
                    if (Character.isHighSurrogate(text.charAt(max - 1))) max--;
                    text = text.substring(0, max);
                }
                v = text;
            }
            final String fv = v;
            return new Action.ValueGetter() {
                public String asTemplateSubstitution() { return StringTag.escapeWithoutQuotes(fv); }
                public Tag asTag() { return StringTag.valueOf(fv); }
            };
        } else if (c instanceof BooleanInput b) {
            boolean on = value == null ? b.initial() : value.asBoolean().orElseThrow();
            return new Action.ValueGetter() {
                public String asTemplateSubstitution() { return on ? b.onTrue() : b.onFalse(); }
                public Tag asTag() { return ByteTag.valueOf(on); }
            };
        } else if (c instanceof SingleOptionInput o) {
            String id = value == null ? o.initial().orElse(o.entries().getFirst()).id() : value.asString().orElseThrow();
            return Action.ValueGetter.of(id);
        } else if (c instanceof NumberRangeInput n) {
            double slider;
            if (value == null) {
                slider = n.rangeInfo().initialSliderValue();
            } else {
                float v = value.asFloat().orElseThrow();
                var r = n.rangeInfo();
                float s = r.start() == r.end() ? 0.5F : Mth.inverseLerp(v, r.start(), r.end());
                slider = Mth.clamp(s, 0.0F, 1.0F);
            }
            String text = (String) sliderString.invoke(null, n, slider);
            float scaled = (Float) sliderScaled.invoke(null, n, slider);
            return new Action.ValueGetter() {
                public String asTemplateSubstitution() { return text; }
                public Tag asTag() { return FloatTag.valueOf(scaled); }
            };
        }
        throw new IllegalArgumentException("input " + input);
    }
}
