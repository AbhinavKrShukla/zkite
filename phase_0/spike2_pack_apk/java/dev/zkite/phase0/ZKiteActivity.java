package dev.zkite.phase0;

import android.app.Activity;
import android.graphics.Color;
import android.os.Bundle;
import android.util.Log;
import android.view.Gravity;
import android.widget.ScrollView;
import android.widget.TextView;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.zip.ZipEntry;
import java.util.zip.ZipInputStream;

/**
 * Phase 0 spike 2: the prebuilt dex shim.
 *
 * <p>Plain {@code android.app.Activity}, not {@code AppCompatActivity}. plan1.md §3 is
 * right that the shipping shim needs AppCompat for IME and insets, but AppCompat means
 * an AndroidX AAR dependency, and dragging one in would make this spike a test of the
 * dependency resolver rather than of the toolchain-free packaging path. What the spike
 * has to prove is that a dex we compiled once, at framework release time, runs inside an
 * APK that pack produced. A plain Activity proves exactly that and nothing else.
 *
 * <p>Everything this class does is observable from {@code adb logcat -s ZKitePhase0}, so
 * the emulator check needs no screenshot and no UI automation.
 */
public class ZKiteActivity extends Activity {
    static final String TAG = "ZKitePhase0";
    static final String STDLIB_ASSET = "python-stdlib.zip";
    static final String STAMP = "3.14.6";

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        // Spike 2's own pass criterion: a pack-built APK installs, launches, and runs
        // our code. True by the time this line executes.
        Log.i(TAG, "ZKITE_SPIKE2_RESULT ok=1 abi=" + android.os.Build.SUPPORTED_ABIS[0]
                + " sdk=" + android.os.Build.VERSION.SDK_INT);

        String pythonLine = bootPython();
        Log.i(TAG, pythonLine);

        TextView tv = new TextView(this);
        tv.setPadding(48, 96, 48, 48);
        tv.setTextSize(14f);
        tv.setTextColor(Color.WHITE);
        tv.setGravity(Gravity.START);
        tv.setText("ZKite phase 0\n\nspike 2: APK built by pack — OK\n\nspike 1:\n" + pythonLine);

        ScrollView sv = new ScrollView(this);
        sv.setBackgroundColor(Color.parseColor("#020617")); // slate-950, per the plan's demo
        sv.addView(tv);
        setContentView(sv);
    }

    private String bootPython() {
        try {
            // libpython first: libzkitespike1.so lists it as DT_NEEDED, and Android's
            // linker will not search the app's lib dir for a transitive dependency by
            // soname alone on every API level we care about.
            System.loadLibrary("python3.14");
            System.loadLibrary("zkitespike1");
        } catch (Throwable t) {
            Log.e(TAG, "loadLibrary failed", t);
            return "ZKITE_SPIKE1_RESULT ok=0 error=" + quote("loadLibrary: " + t);
        }

        File prefix = new File(getFilesDir(), "python");
        try {
            unpackStdlib(prefix);
            linkExtensionModules(prefix);
        } catch (IOException e) {
            Log.e(TAG, "stdlib unpack failed", e);
            return "ZKITE_SPIKE1_RESULT ok=0 error=" + quote("unpack: " + e);
        }

        try {
            return Native.boot(prefix.getAbsolutePath());
        } catch (Throwable t) {
            Log.e(TAG, "Native.boot threw", t);
            return "ZKITE_SPIKE1_RESULT ok=0 error=" + quote("boot: " + t);
        }
    }

    /**
     * Extract {@code assets/python-stdlib.zip} into the app's private storage, once.
     *
     * <p>This is the phase-0 stand-in for what plan1.md §7 describes as staging
     * {@code stdlib.zip}; a real build would keep it zipped and let {@code zipimport}
     * read it in place. Extracting here keeps the failure modes obvious.
     */
    private void unpackStdlib(File prefix) throws IOException {
        File stamp = new File(prefix, ".stamp-" + STAMP);
        if (stamp.exists()) {
            Log.i(TAG, "stdlib already unpacked at " + prefix);
            return;
        }
        long t0 = System.nanoTime();
        deleteRecursively(prefix);
        if (!prefix.mkdirs()) {
            throw new IOException("could not create " + prefix);
        }

        byte[] buf = new byte[64 * 1024];
        int files = 0;
        try (InputStream raw = getAssets().open(STDLIB_ASSET);
             ZipInputStream zis = new ZipInputStream(raw)) {
            ZipEntry entry;
            while ((entry = zis.getNextEntry()) != null) {
                File out = new File(prefix, entry.getName());
                // Guard against a zip entry escaping the target directory.
                if (!out.getCanonicalPath().startsWith(prefix.getCanonicalPath() + File.separator)) {
                    throw new IOException("zip entry outside target: " + entry.getName());
                }
                if (entry.isDirectory()) {
                    out.mkdirs();
                    continue;
                }
                File parent = out.getParentFile();
                if (parent != null) parent.mkdirs();
                try (OutputStream os = new FileOutputStream(out)) {
                    int n;
                    while ((n = zis.read(buf)) > 0) os.write(buf, 0, n);
                }
                files++;
            }
        }
        new FileOutputStream(stamp).close();
        Log.i(TAG, "unpacked " + files + " stdlib files in "
                + ((System.nanoTime() - t0) / 1_000_000) + " ms");
    }

    /**
     * Put CPython's extension modules on {@code sys.path}.
     *
     * <p>They ship in {@code lib/<abi>/} inside the APK, because since Android 10 an app
     * may only {@code dlopen} from its own native library directory — anything unpacked
     * into {@code filesDir} would be unloadable. That directory is only known at runtime,
     * so the path is written into a {@code .pth} file that {@code site} picks up during
     * interpreter startup. No Rust-side change and no hardcoded path.
     */
    private void linkExtensionModules(File prefix) throws IOException {
        File sitePackages = new File(prefix, "lib/python3.14/site-packages");
        if (!sitePackages.isDirectory() && !sitePackages.mkdirs()) {
            throw new IOException("could not create " + sitePackages);
        }
        String nativeLibDir = getApplicationInfo().nativeLibraryDir;
        File pth = new File(sitePackages, "zkite-dynload.pth");
        try (OutputStream os = new FileOutputStream(pth)) {
            os.write((nativeLibDir + "\n").getBytes("UTF-8"));
        }
        Log.i(TAG, "extension modules on sys.path via " + pth + " -> " + nativeLibDir);
    }

    private static void deleteRecursively(File f) {
        File[] kids = f.listFiles();
        if (kids != null) for (File k : kids) deleteRecursively(k);
        f.delete();
    }

    private static String quote(String s) {
        return "\"" + s.replace("\"", "'") + "\"";
    }
}
