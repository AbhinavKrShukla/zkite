package androidx.core;

/**
 * A minimal, hand-written stand-in for the {@code androidx.core.R} class AAPT2
 * normally generates from {@code androidx.core:core}'s own {@code res/values/ids.xml}
 * at ordinary Gradle build time (../../PLAN.md §5, IME). This pipeline has no
 * AAPT2/resource-merging step — see phase_0/README.md's whole "sidestep resource
 * qualifiers" story, and phase_2/PLAN.md §3 doing the same for per-app icons — so
 * when {@code androidx.core.view.ViewCompat}'s static initializer reads dozens of
 * {@code R.id.accessibility_custom_action_N} fields (used purely as {@code
 * View#setTag(int, Object)} keys — opaque, process-local integers, never looked up
 * against {@code resources.arsc}), there is nothing to resolve them, and the class
 * fails to load at all (confirmed live: {@code NoClassDefFoundError: Landroidx/core/
 * R$id;} thrown from {@code ViewCompat.<clinit>}, the first time anything in this
 * app's call graph touched {@code ViewCompat} — which turned out to be
 * {@code vendor/gametextinput}'s own {@code InputConnection.isSoftwareKeyboardVisible()},
 * needed to make {@code Listener.stateChanged} fire at all; see
 * {@code ZKiteSurfaceView.forwardWindowInsets}'s doc comment for the full chain).
 *
 * <p>Traced via {@code javap} against the real
 * {@code androidx.core:core:1.13.1} classes (no source ships in the AAR) to the
 * exact and complete field list {@code ViewCompat.<clinit>} reads — 32 fields,
 * {@code accessibility_custom_action_0} through {@code _31}, nothing else. The
 * values below are arbitrary but distinct 32-bit ints in the AAPT resource-id
 * package range (0x7f, matching how a real app's own resources are packaged) —
 * correctness here only requires uniqueness (they're never compared against a real
 * resource table), not matching whatever AAPT2 would actually have assigned.
 *
 * <p>If a future dependency on more of {@code androidx.core} needs additional
 * {@code R} fields, extend this class the same way: {@code javap} the real class,
 * find the exact {@code NoClassDefFoundError}/{@code ClassNotFoundException} target,
 * add just that field — not a speculative full port of every id androidx.core
 * happens to declare.
 */
public final class R {
    public static final class id {
        public static final int accessibility_custom_action_0 = 0x7f0a0000;
        public static final int accessibility_custom_action_1 = 0x7f0a0001;
        public static final int accessibility_custom_action_2 = 0x7f0a0002;
        public static final int accessibility_custom_action_3 = 0x7f0a0003;
        public static final int accessibility_custom_action_4 = 0x7f0a0004;
        public static final int accessibility_custom_action_5 = 0x7f0a0005;
        public static final int accessibility_custom_action_6 = 0x7f0a0006;
        public static final int accessibility_custom_action_7 = 0x7f0a0007;
        public static final int accessibility_custom_action_8 = 0x7f0a0008;
        public static final int accessibility_custom_action_9 = 0x7f0a0009;
        public static final int accessibility_custom_action_10 = 0x7f0a000a;
        public static final int accessibility_custom_action_11 = 0x7f0a000b;
        public static final int accessibility_custom_action_12 = 0x7f0a000c;
        public static final int accessibility_custom_action_13 = 0x7f0a000d;
        public static final int accessibility_custom_action_14 = 0x7f0a000e;
        public static final int accessibility_custom_action_15 = 0x7f0a000f;
        public static final int accessibility_custom_action_16 = 0x7f0a0010;
        public static final int accessibility_custom_action_17 = 0x7f0a0011;
        public static final int accessibility_custom_action_18 = 0x7f0a0012;
        public static final int accessibility_custom_action_19 = 0x7f0a0013;
        public static final int accessibility_custom_action_20 = 0x7f0a0014;
        public static final int accessibility_custom_action_21 = 0x7f0a0015;
        public static final int accessibility_custom_action_22 = 0x7f0a0016;
        public static final int accessibility_custom_action_23 = 0x7f0a0017;
        public static final int accessibility_custom_action_24 = 0x7f0a0018;
        public static final int accessibility_custom_action_25 = 0x7f0a0019;
        public static final int accessibility_custom_action_26 = 0x7f0a001a;
        public static final int accessibility_custom_action_27 = 0x7f0a001b;
        public static final int accessibility_custom_action_28 = 0x7f0a001c;
        public static final int accessibility_custom_action_29 = 0x7f0a001d;
        public static final int accessibility_custom_action_30 = 0x7f0a001e;
        public static final int accessibility_custom_action_31 = 0x7f0a001f;
    }
}
