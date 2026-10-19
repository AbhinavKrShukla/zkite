package dev.zkite.runtime;

import android.Manifest;
import android.app.*;
import android.content.*;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.hardware.*;
import android.hardware.camera2.CameraManager;
import android.media.*;
import android.net.*;
import android.os.*;
import android.provider.*;
import android.speech.tts.*;
import android.view.*;
import org.json.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.*;

/** Main-thread-owned Android services, with bounded I/O and coalesced sensor events.
 * exchange() never waits for the UI or Python. No Python callback runs on Android UI.
 */
public final class DeviceServices {
    private static volatile DeviceServices current;
    private final Activity activity;
    private final Handler ui = new Handler(Looper.getMainLooper());
    private final ThreadPoolExecutor io = new ThreadPoolExecutor(2, 2, 30, TimeUnit.SECONDS,
        new ArrayBlockingQueue<Runnable>(16));
    private final ConcurrentHashMap<Long, JSONObject> results = new ConcurrentHashMap<>();
    private final ConcurrentHashMap<Long, JSONObject> samples = new ConcurrentHashMap<>();
    private final Set<Long> pending = ConcurrentHashMap.newKeySet();
    private final Map<Long, SensorWatch> watches = new HashMap<>();
    private final Map<Long, Sound> sounds = new HashMap<>();
    private static final java.util.concurrent.atomic.AtomicLong NEXT_HANDLE = new java.util.concurrent.atomic.AtomicLong(1);
    private JSONObject interactive;
    private String waitingPermission;
    private File captureFile;
    private boolean foreground;
    private volatile boolean destroyed;
    private TextToSpeech tts;
    private long speechId;
    private boolean ttsReady;
    private final Sound speechFocus = new Sound(-1);
    private final BroadcastReceiver noisy = new BroadcastReceiver() {
        @Override public void onReceive(Context context, Intent intent) {
            for (Sound sound : sounds.values()) if (sound.player != null) {
                try { if (sound.player.isPlaying()) sound.player.pause(); sound.state="paused"; sound.reason="route_disconnected"; sound.abandonFocus(); }
                catch (Exception ignored) { }
            }
        }
    };
    private String torchId;
    private JSONObject resumeRequest;
    private final Map<Long, android.location.LocationListener> locations = new HashMap<>();
    private final Map<Long, CancellationSignal> authentication = new HashMap<>();
    private final JSONObject info;
    private static final int REQUEST = 24610, PERMISSION = 24611;

    public DeviceServices(Activity activity) {
        this.activity = activity;
        File data = new File(activity.getFilesDir(), "user-data"); data.mkdirs();
        info = obj("version", 1, "sdk", Build.VERSION.SDK_INT, "model", Build.MODEL,
            "data_dir", data.getAbsolutePath(), "cache_dir", activity.getCacheDir().getAbsolutePath());
        if (Build.VERSION.SDK_INT >= 33) activity.registerReceiver(noisy,new IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY),Context.RECEIVER_NOT_EXPORTED);
        else activity.registerReceiver(noisy,new IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY));
        current = this;
    }
    private static JSONObject obj(Object... kv) {
        JSONObject out = new JSONObject();
        try { for (int i=0; i<kv.length; i+=2) out.put((String)kv[i], kv[i+1] == null ? JSONObject.NULL : kv[i+1]); }
        catch (JSONException e) { throw new IllegalArgumentException(e); }
        return out;
    }
    public static String exchange(String input) {
        DeviceServices host = current;
        if (host == null || host.destroyed) return obj("error", "DeviceUnavailable", "message", "Android Activity unavailable").toString();
        try {
            JSONObject q = new JSONObject(input);
            String op = q.getString("op");
            long id = q.optLong("id");
            if (op.equals("info")) return host.info.toString();
            if (op.equals("poll")) {
                JSONArray events = new JSONArray();
                for (Long key : host.results.keySet()) {
                    JSONObject value = host.results.remove(key);
                    if (value != null) { events.put(value); host.pending.remove(key); }
                }
                for (Long key : host.samples.keySet()) {
                    JSONObject value = host.samples.remove(key);
                    if (value != null) events.put(value);
                }
                return events.toString();
            }
            if (op.equals("cancel")) {
                host.pending.remove(id); host.results.remove(id);
                host.ui.post(() -> host.cancel(id));
                return "null";
            }
            if (op.equals("unsubscribe")) { host.ui.post(() -> host.unsubscribe(id)); return "null"; }
            if (op.equals("reset")) { host.pending.clear(); host.results.clear(); host.ui.post(host::reset); return "null"; }
            if (q.optInt("version") != 1) return obj("error", "RuntimeMismatch", "message", "Device protocol mismatch").toString();
            if (host.pending.size() >= 32) return obj("error", "ResourceBusy", "message", "Android service queue full").toString();
            host.pending.add(id);
            host.ui.post(() -> { if (host.pending.contains(id)) host.execute(q); });
            return "null";
        } catch (Exception e) { return obj("error", "DeviceError", "message", e.toString()).toString(); }
    }
    private void done(long id, Object value) {
        if (pending.contains(id)) results.put(id, obj("id", id, "value", value));
    }
    private void fail(long id, String code, String message) {
        if (pending.contains(id)) results.put(id, obj("id", id, "error", code, "message", message));
    }
    private void failure(long id, Exception e) {
        fail(id, e instanceof SecurityException ? "PermissionDenied" :
            e instanceof ActivityNotFoundException ? "UnsupportedFeature" :
            e instanceof RejectedExecutionException ? "ResourceBusy" : "DeviceIOError", e.getMessage() == null ? e.toString() : e.getMessage());
    }
    private boolean declared(String permission) {
        try {
            String[] declared = activity.getPackageManager().getPackageInfo(activity.getPackageName(), PackageManager.GET_PERMISSIONS).requestedPermissions;
            return declared != null && Arrays.asList(declared).contains(permission);
        } catch (Exception e) { return false; }
    }
    private boolean ensure(JSONObject q, String permission) {
        long id = q.optLong("id");
        if (!declared(permission)) { fail(id, "PermissionNotDeclared", "Add " + permission + " to android.permissions or capabilities, then rebuild"); return false; }
        if (activity.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED) return true;
        if (interactive != null && interactive != q) { fail(id, "ResourceBusy", "Another system dialog is open"); return false; }
        interactive = q; waitingPermission = permission;
        activity.requestPermissions(new String[]{permission}, PERMISSION);
        return false;
    }
    public void onPermissions(int code, String[] permissions, int[] grants) {
        if (code != PERMISSION || interactive == null) return;
        JSONObject q = interactive; interactive = null; waitingPermission = null;
        if (!pending.contains(q.optLong("id"))) return;
        if (grants.length > 0 && grants[0] == PackageManager.PERMISSION_GRANTED) {
            if (foreground) execute(q); else resumeRequest = q;
        }
        else fail(q.optLong("id"), "PermissionDenied", "Permission denied; enable it in app settings if needed");
    }
    private void launch(JSONObject q, Intent intent) {
        if (interactive != null) { fail(q.optLong("id"), "ResourceBusy", "Another system dialog is open"); return; }
        interactive = q;
        try { activity.startActivityForResult(intent, REQUEST); }
        catch (Exception e) { interactive = null; failure(q.optLong("id"), e); }
    }
    private Uri uri(String value) { return Uri.parse(value); }
    private InputStream input(Uri uri) throws IOException {
        if ("file".equals(uri.getScheme())) return new FileInputStream(new File(uri.getPath()));
        InputStream stream = activity.getContentResolver().openInputStream(uri);
        if (stream == null) throw new IOException("Provider returned no stream");
        return stream;
    }
    private JSONObject describe(Uri u) throws Exception {
        String name = "document", type = activity.getContentResolver().getType(u);
        Long size = null;
        String path = null;
        if ("file".equals(u.getScheme())) {
            File f = new File(u.getPath()); name = f.getName(); size = f.length(); path = f.getAbsolutePath();
            type = java.net.URLConnection.guessContentTypeFromName(name);
        } else {
            try (Cursor c = activity.getContentResolver().query(u, new String[]{OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE}, null, null, null)) {
                if (c != null && c.moveToFirst()) {
                    int n = c.getColumnIndex(OpenableColumns.DISPLAY_NAME), s = c.getColumnIndex(OpenableColumns.SIZE);
                    if (n >= 0 && !c.isNull(n)) name = c.getString(n);
                    if (s >= 0 && !c.isNull(s)) size = c.getLong(s);
                }
            }
        }
        return obj("uri", u.toString(), "name", name, "mime_type", type == null ? "application/octet-stream" : type, "size", size, "path", path);
    }
    private void transfer(long id, Uri source, OutputStream out, long max) throws Exception {
        try (InputStream in = input(source)) {
            byte[] buffer = new byte[65536]; long total = 0; int n;
            while ((n = in.read(buffer)) != -1) {
                if (!pending.contains(id) || destroyed) throw new IOException("Transfer cancelled");
                total += n;
                if (total > max) throw new IOException("File exceeds max_bytes");
                out.write(buffer, 0, n);
            }
            out.flush();
        }
    }
    public void onActivityResult(int code, int result, Intent data) {
        if (code != REQUEST || interactive == null) return;
        JSONObject q = interactive; interactive = null;
        long id = q.optLong("id"); String op = q.optString("op");
        File captured = captureFile; captureFile = null;
        if (!pending.contains(id) || result != Activity.RESULT_OK) {
            if (captured != null) captured.delete();
            done(id, q.optJSONObject("args").optBoolean("multiple") ? new JSONArray() : null);
            return;
        }
        try {
            io.execute(() -> {
                try {
                    JSONObject args = q.getJSONObject("args");
                    if (op.startsWith("camera.")) {
                        if (captured == null || captured.length() == 0) throw new IOException("Camera returned no media");
                        done(id, describe(Uri.fromFile(captured))); return;
                    }
                    if (data == null) throw new IOException("Picker returned no document");
                    if (op.equals("files.save")) {
                        Uri destination = data.getData();
                        try (OutputStream out = activity.getContentResolver().openOutputStream(destination, "wt")) {
                            if (out == null) throw new IOException("Cannot open destination");
                            transfer(id, uri(args.getString("source")), out, 256L * 1024 * 1024);
                        }
                        done(id, describe(destination)); return;
                    }
                    JSONArray selected = new JSONArray();
                    int limit = args.optInt("limit", 1);
                    if (data.getClipData() != null) {
                        if (data.getClipData().getItemCount() > limit) throw new IOException("Selection exceeds requested limit");
                        for (int i=0; i<data.getClipData().getItemCount(); i++) selected.put(describe(data.getClipData().getItemAt(i).getUri()));
                    } else if (data.getData() != null) selected.put(describe(data.getData()));
                    done(id, args.optBoolean("multiple") ? selected : selected.opt(0));
                } catch (Exception e) { if (captured != null) captured.delete(); failure(id, e); }
            });
        } catch (Exception e) { failure(id, e); }
    }
    private void execute(JSONObject q) {
        long id = q.optLong("id"); String op = q.optString("op"); JSONObject a = q.optJSONObject("args");
        try {
            if (!foreground && !op.startsWith("files.copy") && !op.equals("audio.close")) {
                fail(id, "OperationInterrupted", "Device operation requires a foreground Activity"); return;
            }
            switch (op) {
                case "permissions.status": case "permissions.ask": {
                    String p = a.getString("permission"); if (!p.contains(".")) p = "android.permission." + p;
                    if (op.endsWith("ask") && !ensure(q, p)) return;
                    done(id, obj("declared", declared(p), "granted", activity.checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED,
                        "rationale", activity.shouldShowRequestPermissionRationale(p))); return;
                }
                case "files.pick": case "files.photo": {
                    Intent intent;
                    if (op.equals("files.photo") && Build.VERSION.SDK_INT >= 33) intent = new Intent(MediaStore.ACTION_PICK_IMAGES).setType("image/*");
                    else {
                        intent = new Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*");
                        if (op.equals("files.photo")) intent.setType("image/*");
                        else {
                            JSONArray types = a.getJSONArray("types"); String[] mime = new String[types.length()];
                            for (int i=0; i<mime.length; i++) mime[i] = types.getString(i);
                            intent.putExtra(Intent.EXTRA_MIME_TYPES, mime).putExtra(Intent.EXTRA_ALLOW_MULTIPLE, a.optBoolean("multiple"));
                        }
                        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
                    }
                    launch(q, intent); return;
                }
                case "files.save":
                    launch(q, new Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType(a.getString("mime")).putExtra(Intent.EXTRA_TITLE, a.getString("name"))); return;
                case "files.copy": {
                    io.execute(() -> {
                        File tmp = null;
                        try {
                            File target = new File(a.getString("destination"));
                            tmp = File.createTempFile(".zkite-copy-", ".tmp", target.getParentFile());
                            try (OutputStream out = new FileOutputStream(tmp)) { transfer(id, uri(a.getString("uri")), out, a.getLong("max_bytes")); }
                            if (!pending.contains(id)) throw new IOException("Transfer cancelled");
                            Files.move(tmp.toPath(), target.toPath(), StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
                            done(id, target.getAbsolutePath());
                        } catch (Exception e) { failure(id, e); }
                        finally { if (tmp != null) tmp.delete(); }
                    }); return;
                }
                case "files.persist":
                    activity.getContentResolver().takePersistableUriPermission(uri(a.getString("uri")), Intent.FLAG_GRANT_READ_URI_PERMISSION); done(id, null); return;
                case "files.release":
                    activity.getContentResolver().releasePersistableUriPermission(uri(a.getString("uri")), Intent.FLAG_GRANT_READ_URI_PERMISSION); done(id, null); return;
                case "camera.capture": case "camera.video": {
                    if (declared(Manifest.permission.CAMERA) && !ensure(q, Manifest.permission.CAMERA)) return;
                    if (interactive != null) { fail(id, "ResourceBusy", "Another system dialog is open"); return; }
                    captureFile = File.createTempFile("capture-", op.endsWith("video") ? ".mp4" : ".jpg", SharedFiles.root(activity));
                    Uri output = SharedFiles.uri(activity, captureFile);
                    Intent intent = new Intent(op.endsWith("video") ? MediaStore.ACTION_VIDEO_CAPTURE : MediaStore.ACTION_IMAGE_CAPTURE);
                    intent.putExtra(MediaStore.EXTRA_OUTPUT, output).addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION | Intent.FLAG_GRANT_READ_URI_PERMISSION);
                    intent.setClipData(ClipData.newRawUri("capture", output));
                    if (op.endsWith("video")) intent.putExtra(MediaStore.EXTRA_DURATION_LIMIT, a.optInt("duration", 60));
                    launch(q, intent); return;
                }
                case "camera.torch": {
                    if (!ensure(q, Manifest.permission.CAMERA)) return;
                    CameraManager manager = (CameraManager)activity.getSystemService(Context.CAMERA_SERVICE);
                    if (torchId == null) for (String camera : manager.getCameraIdList()) {
                        if (Boolean.TRUE.equals(manager.getCameraCharacteristics(camera).get(android.hardware.camera2.CameraCharacteristics.FLASH_INFO_AVAILABLE))) { torchId = camera; break; }
                    }
                    if (torchId == null) { fail(id, "UnsupportedFeature", "No camera flash"); return; }
                    manager.setTorchMode(torchId, a.getBoolean("enabled")); done(id, null); return;
                }
                case "haptics.perform": {
                    String effect = a.getString("effect");
                    int feedback = HapticFeedbackConstants.CONTEXT_CLICK;
                    if (effect.equals("long_press")) feedback = HapticFeedbackConstants.LONG_PRESS;
                    if (effect.equals("keyboard")) feedback = HapticFeedbackConstants.KEYBOARD_TAP;
                    if (Build.VERSION.SDK_INT >= 30 && effect.equals("confirm")) feedback = HapticFeedbackConstants.CONFIRM;
                    if (Build.VERSION.SDK_INT >= 30 && effect.equals("reject")) feedback = HapticFeedbackConstants.REJECT;
                    done(id, activity.getWindow().getDecorView().performHapticFeedback(feedback)); return;
                }
                case "haptics.vibrate": case "haptics.stop": {
                    if (!ensure(q, Manifest.permission.VIBRATE)) return;
                    Vibrator vibrator = (Vibrator)activity.getSystemService(Context.VIBRATOR_SERVICE);
                    if (op.endsWith("stop")) vibrator.cancel();
                    else {
                        if (!vibrator.hasVibrator()) { fail(id, "UnsupportedFeature", "No vibrator"); return; }
                        JSONArray p = a.getJSONArray("pattern"); long[] timings = new long[p.length()];
                        for (int i=0; i<timings.length; i++) timings[i] = p.getLong(i);
                        vibrator.vibrate(VibrationEffect.createWaveform(timings, -1));
                    }
                    done(id, null); return;
                }
                case "sensors.list": {
                    SensorManager manager = (SensorManager)activity.getSystemService(Context.SENSOR_SERVICE);
                    JSONArray available = new JSONArray();
                    for (String name : new String[]{"accelerometer", "linear_acceleration", "gyroscope", "rotation", "light", "proximity", "magnetometer", "shake"})
                        if (manager.getDefaultSensor(sensorType(name)) != null) available.put(name);
                    done(id, available); return;
                }
                case "sensors.watch": {
                    if (watches.size() >= 16) { fail(id, "ResourceBusy", "At most 16 sensor subscriptions"); return; }
                    long handle = NEXT_HANDLE.getAndIncrement();
                    SensorWatch watch = new SensorWatch(handle, a.getString("sensor"), a.getDouble("rate_hz"));
                    if (watch.sensor == null) { fail(id, "UnsupportedFeature", "Sensor unavailable"); return; }
                    watch.requestId=id; watches.put(handle, watch); watch.start(); done(id, handle); return;
                }
                case "audio.record": {
                    if (sounds.size() >= 8) { fail(id,"ResourceBusy","Too many audio handles; close or stop previous handles"); return; }
                    if (!ensure(q, Manifest.permission.RECORD_AUDIO)) return;
                    for (Sound sound : sounds.values()) if (sound.recorder != null) { fail(id, "ResourceBusy", "Microphone already recording"); return; }
                    Sound sound = new Sound(NEXT_HANDLE.getAndIncrement()); sound.requestId=id; sound.file = File.createTempFile("recording-", ".m4a", SharedFiles.root(activity));
                    sounds.put(sound.handle, sound);
                    try {
                        sound.recorder = new MediaRecorder();
                        sound.recorder.setAudioSource(MediaRecorder.AudioSource.MIC);
                        sound.recorder.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4);
                        sound.recorder.setAudioEncoder(MediaRecorder.AudioEncoder.AAC);
                        sound.recorder.setAudioEncodingBitRate(128000);
                        sound.recorder.setAudioSamplingRate(44100);
                        sound.recorder.setOutputFile(sound.file.getAbsolutePath());
                        sound.recorder.setMaxDuration(a.getInt("max_seconds") * 1000);
                        sound.recorder.setMaxFileSize(a.getLong("max_bytes"));
                        sound.recorder.setOnInfoListener((r, what, extra) -> { if (what == MediaRecorder.MEDIA_RECORDER_INFO_MAX_DURATION_REACHED || what == MediaRecorder.MEDIA_RECORDER_INFO_MAX_FILESIZE_REACHED) sound.finishRecording("limit"); });
                        sound.recorder.setOnErrorListener((r, what, extra) -> sound.finishRecording("error"));
                        sound.recorder.prepare(); sound.recorder.start(); sound.started = SystemClock.elapsedRealtime(); sound.state = "recording";
                        done(id, sound.handle);
                    } catch (Exception e) { sound.close(); sounds.remove(sound.handle); throw e; }
                    return;
                }
                case "audio.record.stop": {
                    Sound sound = sound(a); sound.finishRecording("stopped");
                    if (!sound.valid) { sound.close(); sounds.remove(sound.handle); throw new IOException("Recording could not be finalized"); }
                    done(id, describe(Uri.fromFile(sound.file))); sounds.remove(sound.handle); return;
                }
                case "audio.play": {
                    if (sounds.size() >= 8) { fail(id, "ResourceBusy", "Too many audio handles"); return; }
                    Sound sound = new Sound(NEXT_HANDLE.getAndIncrement()); sound.requestId=id; sounds.put(sound.handle, sound);
                    sound.player = new MediaPlayer();
                    sound.player.setAudioAttributes(new AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build());
                    sound.player.setDataSource(activity, uri(a.getString("uri")));
                    sound.player.setVolume((float)a.getDouble("volume"), (float)a.getDouble("volume"));
                    sound.player.setLooping(a.optBoolean("loop"));
                    sound.player.setOnPreparedListener(p -> {
                        if (!pending.contains(id) || !foreground) { sound.close(); sounds.remove(sound.handle); fail(id,"OperationInterrupted","Playback interrupted before preparation completed"); return; }
                        if (!sound.focus()) { sound.close(); sounds.remove(sound.handle); fail(id, "ResourceBusy", "Audio focus unavailable"); return; }
                        p.start(); sound.state = "playing"; done(id, sound.handle);
                    });
                    sound.player.setOnCompletionListener(p -> { sound.state = "completed"; sound.abandonFocus(); });
                    sound.player.setOnErrorListener((p,w,e) -> { sound.state = "error"; sound.close(); fail(id,"DeviceIOError","Playback failed"); return true; });
                    sound.player.prepareAsync(); return;
                }
                case "audio.pause": { Sound sound = sound(a); sound.player.pause(); sound.state = "paused"; sound.abandonFocus(); done(id, null); return; }
                case "audio.resume": { Sound sound = sound(a); if (!sound.focus()) { fail(id,"ResourceBusy","Audio focus unavailable"); return; } sound.player.start(); sound.state = "playing"; done(id,null); return; }
                case "audio.seek": { Sound sound = sound(a); sound.player.seekTo((int)(a.getDouble("seconds") * 1000)); done(id,null); return; }
                case "audio.close": { Sound sound = sounds.remove(a.getLong("handle")); if (sound != null) sound.close(); done(id,null); return; }
                case "audio.state": { Sound sound = sound(a); done(id,obj("state",sound.state,"reason",sound.reason,"position",sound.player != null ? sound.player.getCurrentPosition()/1000.0 : (SystemClock.elapsedRealtime()-sound.started)/1000.0)); return; }
                case "audio.speak": speak(q); return;
                case "clipboard.write":
                    ((ClipboardManager)activity.getSystemService(Context.CLIPBOARD_SERVICE)).setPrimaryClip(ClipData.newPlainText("",a.getString("text"))); done(id,null); return;
                case "clipboard.read": {
                    ClipboardManager clipboard = (ClipboardManager)activity.getSystemService(Context.CLIPBOARD_SERVICE);
                    ClipData clip = clipboard.getPrimaryClip(); done(id,clip == null || clip.getItemCount()==0 ? null : clip.getItemAt(0).coerceToText(activity).toString()); return;
                }
                case "share.send": share(q); return;
                case "system.open": activity.startActivity(new Intent(Intent.ACTION_VIEW,uri(a.getString("url")))); done(id,null); return;
                case "system.settings": activity.startActivity(new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS,uri("package:"+activity.getPackageName()))); done(id,null); return;
                case "system.awake":
                    if (a.getBoolean("enabled")) activity.getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                    else activity.getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                    done(id,null); return;
                case "system.battery": {
                    Intent battery = activity.registerReceiver(null,new IntentFilter(Intent.ACTION_BATTERY_CHANGED));
                    int scale=battery.getIntExtra(BatteryManager.EXTRA_SCALE,100), level=battery.getIntExtra(BatteryManager.EXTRA_LEVEL,-1);
                    done(id,obj("level",level < 0 || scale <= 0 ? null : (double)level/scale,"charging",battery.getIntExtra(BatteryManager.EXTRA_PLUGGED,0)!=0)); return;
                }
                case "system.network": {
                    if (!ensure(q,Manifest.permission.ACCESS_NETWORK_STATE)) return;
                    ConnectivityManager manager=(ConnectivityManager)activity.getSystemService(Context.CONNECTIVITY_SERVICE);
                    NetworkCapabilities caps=manager.getNetworkCapabilities(manager.getActiveNetwork());
                    done(id,obj("connected",caps!=null,"validated",caps!=null && caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED),"metered",manager.isActiveNetworkMetered())); return;
                }
                case "location.current": locate(q); return;
                case "notifications.show": {
                    if (Build.VERSION.SDK_INT >= 33 && !ensure(q, Manifest.permission.POST_NOTIFICATIONS)) return;
                    NotificationManager manager = (NotificationManager)activity.getSystemService(Context.NOTIFICATION_SERVICE);
                    if (!manager.areNotificationsEnabled()) { fail(id,"PermissionDenied","Notifications are disabled in settings"); return; }
                    String channel = a.getString("channel");
                    manager.createNotificationChannel(new NotificationChannel(channel,a.getString("channel_name"),NotificationManager.IMPORTANCE_DEFAULT));
                    Notification notification = new Notification.Builder(activity,channel)
                        .setSmallIcon(android.R.drawable.ic_dialog_info).setContentTitle(a.getString("title"))
                        .setContentText(a.getString("body")).setAutoCancel(true).build();
                    manager.notify(a.getInt("notification_id"),notification); done(id,null); return;
                }
                case "notifications.cancel":
                    ((NotificationManager)activity.getSystemService(Context.NOTIFICATION_SERVICE)).cancel(a.getInt("notification_id")); done(id,null); return;
                case "secrets.set": case "secrets.get": case "secrets.delete": secret(q); return;
                case "biometrics.authenticate": {
                    if (Build.VERSION.SDK_INT < 28) { fail(id,"UnsupportedFeature","Biometric prompt requires Android 9+"); return; }
                    if (interactive != null || !authentication.isEmpty()) { fail(id,"ResourceBusy","Another authentication or system dialog is open"); return; }
                    CancellationSignal signal = new CancellationSignal(); authentication.put(id,signal);
                    android.hardware.biometrics.BiometricPrompt prompt = new android.hardware.biometrics.BiometricPrompt.Builder(activity)
                        .setTitle(a.getString("title")).setNegativeButton("Cancel",activity.getMainExecutor(),(dialog,which) -> {
                            authentication.remove(id); fail(id,"PermissionDenied","Authentication cancelled");
                        }).build();
                    prompt.authenticate(signal,activity.getMainExecutor(),new android.hardware.biometrics.BiometricPrompt.AuthenticationCallback() {
                        @Override public void onAuthenticationSucceeded(android.hardware.biometrics.BiometricPrompt.AuthenticationResult result) {
                            authentication.remove(id); done(id,true);
                        }
                        @Override public void onAuthenticationError(int code,CharSequence message) {
                            authentication.remove(id); fail(id,"PermissionDenied",message.toString());
                        }
                    }); return;
                }
                default: fail(id,"UnsupportedFeature","Unknown device operation: "+op);
            }
        } catch (Exception e) {
            for(Sound sound:new ArrayList<>(sounds.values())) if(sound.requestId==id) { sound.close(); sounds.remove(sound.handle); }
            for(SensorWatch watch:new ArrayList<>(watches.values())) if(watch.requestId==id) unsubscribe(watch.handle);
            failure(id,e);
        }
    }
    private void locate(JSONObject q) throws Exception {
        long id=q.getLong("id"); JSONObject a=q.getJSONObject("args");
        if (!ensure(q,Manifest.permission.ACCESS_COARSE_LOCATION)) return;
        if (a.optBoolean("high_accuracy") && !q.optBoolean("location_asked") && activity.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION)!=PackageManager.PERMISSION_GRANTED) {
            if (!declared(Manifest.permission.ACCESS_FINE_LOCATION)) { fail(id,"PermissionNotDeclared","Declare location capability for high accuracy"); return; }
            if (interactive!=null) { fail(id,"ResourceBusy","Another dialog is open"); return; }
            q.put("location_asked",true); interactive=q; waitingPermission=Manifest.permission.ACCESS_FINE_LOCATION;
            activity.requestPermissions(new String[]{Manifest.permission.ACCESS_COARSE_LOCATION,Manifest.permission.ACCESS_FINE_LOCATION},PERMISSION); return;
        }
        android.location.LocationManager manager=(android.location.LocationManager)activity.getSystemService(Context.LOCATION_SERVICE);
        boolean fine=activity.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION)==PackageManager.PERMISSION_GRANTED;
        String provider=fine && a.optBoolean("high_accuracy") && manager.isProviderEnabled(android.location.LocationManager.GPS_PROVIDER)
            ? android.location.LocationManager.GPS_PROVIDER : android.location.LocationManager.NETWORK_PROVIDER;
        if (!manager.isProviderEnabled(provider)) { fail(id,"DeviceUnavailable","Location provider disabled or unavailable"); return; }
        android.location.LocationListener listener=new android.location.LocationListener() {
            @Override public void onLocationChanged(android.location.Location value) {
                stopLocation(id);
                done(id,obj("latitude",value.getLatitude(),"longitude",value.getLongitude(),"accuracy",value.getAccuracy(),
                    "timestamp_ms",value.getTime(),"altitude",value.hasAltitude() ? value.getAltitude() : null));
            }
            @Override public void onProviderDisabled(String provider) { stopLocation(id); fail(id,"DeviceUnavailable","Location provider disabled"); }
            @Override public void onProviderEnabled(String provider) { }
            @Override public void onStatusChanged(String provider,int status,Bundle extras) { }
        };
        locations.put(id,listener); manager.requestLocationUpdates(provider,0,0,listener,Looper.getMainLooper());
    }
    private void stopLocation(long id) {
        android.location.LocationListener listener=locations.remove(id);
        if(listener!=null) ((android.location.LocationManager)activity.getSystemService(Context.LOCATION_SERVICE)).removeUpdates(listener);
    }
    private void secret(JSONObject q) {
        long id=q.optLong("id");
        io.execute(() -> {
            synchronized (DeviceServices.class) {
                try {
                    JSONObject args=q.getJSONObject("args");
                    byte[] digest=java.security.MessageDigest.getInstance("SHA-256").digest(args.getString("name").getBytes(java.nio.charset.StandardCharsets.UTF_8));
                    String name=android.util.Base64.encodeToString(digest,android.util.Base64.NO_WRAP|android.util.Base64.URL_SAFE);
                    File directory=new File(activity.getNoBackupFilesDir(),"zkite-secrets"); directory.mkdirs();
                    android.util.AtomicFile file=new android.util.AtomicFile(new File(directory,name));
                    if(q.getString("op").equals("secrets.delete")) { file.delete(); done(id,null); return; }
                    if(q.getString("op").equals("secrets.get") && !file.getBaseFile().exists()) { done(id,null); return; }
                    java.security.KeyStore keys=java.security.KeyStore.getInstance("AndroidKeyStore"); keys.load(null);
                    String alias="zkite.secrets.v1";
                    if(!keys.containsAlias(alias)) {
                        if(q.getString("op").equals("secrets.get")) throw new IOException("Encryption key is unavailable");
                        javax.crypto.KeyGenerator generator=javax.crypto.KeyGenerator.getInstance("AES","AndroidKeyStore");
                        generator.init(new android.security.keystore.KeyGenParameterSpec.Builder(alias,
                            android.security.keystore.KeyProperties.PURPOSE_ENCRYPT|android.security.keystore.KeyProperties.PURPOSE_DECRYPT)
                            .setBlockModes("GCM").setEncryptionPaddings("NoPadding").build()); generator.generateKey();
                    }
                    javax.crypto.SecretKey key=(javax.crypto.SecretKey)keys.getKey(alias,null);
                    javax.crypto.Cipher cipher=javax.crypto.Cipher.getInstance("AES/GCM/NoPadding");
                    if(q.getString("op").equals("secrets.set")) {
                        cipher.init(javax.crypto.Cipher.ENCRYPT_MODE,key); cipher.updateAAD(digest);
                        byte[] encrypted=cipher.doFinal(args.getString("value").getBytes(java.nio.charset.StandardCharsets.UTF_8));
                        FileOutputStream stream=null;
                        try { stream=file.startWrite(); stream.write(cipher.getIV()); stream.write(encrypted); file.finishWrite(stream); }
                        catch(Exception e) { if(stream!=null) file.failWrite(stream); throw e; }
                        done(id,null);
                    } else {
                        byte[] stored=file.readFully();
                        if(stored.length<28) throw new IOException("Invalid encrypted value");
                        cipher.init(javax.crypto.Cipher.DECRYPT_MODE,key,new javax.crypto.spec.GCMParameterSpec(128,Arrays.copyOfRange(stored,0,12)));
                        cipher.updateAAD(digest);
                        done(id,new String(cipher.doFinal(Arrays.copyOfRange(stored,12,stored.length)),java.nio.charset.StandardCharsets.UTF_8));
                    }
                } catch(Exception e) { failure(id,e); }
            }
        });
    }
    private Sound sound(JSONObject a) throws Exception {
        Sound sound = sounds.get(a.getLong("handle"));
        if (sound == null) throw new IOException("Audio handle closed or interrupted");
        return sound;
    }
    private final class Sound {
        final long handle;
        MediaPlayer player;
        MediaRecorder recorder;
        File file;
        boolean valid;
        long started;
        String state="preparing", reason="";
        AudioFocusRequest focus;
        long requestId;
        Sound(long handle) { this.handle=handle; }
        boolean focus() {
            AudioManager manager=(AudioManager)activity.getSystemService(Context.AUDIO_SERVICE);
            if (focus == null) focus=new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                .setAudioAttributes(new AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).build())
                .setOnAudioFocusChangeListener(change -> {
                    if (change < 0 && handle == -1 && speechId != 0) {
                        if(tts!=null) tts.stop(); fail(speechId,"OperationInterrupted","Speech lost audio focus"); speechId=0;
                    }
                    if (change < 0 && player != null) {
                        try { if (player.isPlaying()) player.pause(); state="paused"; reason="focus_lost"; }
                        catch (Exception ignored) { }
                    }
                },ui).build();
            return manager.requestAudioFocus(focus)==AudioManager.AUDIOFOCUS_REQUEST_GRANTED;
        }
        void abandonFocus() {
            if (focus != null) ((AudioManager)activity.getSystemService(Context.AUDIO_SERVICE)).abandonAudioFocusRequest(focus);
        }
        void finishRecording(String why) {
            if (recorder == null) return;
            try { recorder.stop(); valid=file.length()>0; }
            catch (Exception e) { valid=false; }
            finally { recorder.release(); recorder=null; state=valid ? "stopped" : "error"; reason=why; }
        }
        void close() {
            finishRecording("cancelled");
            if (player != null) { player.release(); player=null; }
            abandonFocus();
            if (file != null) file.delete();
            state="closed";
        }
    }
    private void speak(JSONObject q) throws Exception {
        long id=q.getLong("id");
        if (speechId != 0) { fail(id,"ResourceBusy","Speech already in progress"); return; }
        speechId=id;
        if (tts == null) {
            tts=new TextToSpeech(activity,status -> ui.post(() -> {
                if (status != TextToSpeech.SUCCESS) { fail(speechId,"UnsupportedFeature","Text-to-speech engine unavailable"); speechId=0; return; }
                ttsReady=true;
                try { startSpeech(q); } catch (Exception e) { failure(id,e); speechId=0; }
            }));
        } else if (ttsReady) startSpeech(q);
        else { fail(id,"ResourceBusy","Speech engine is initializing"); speechId=0; }
    }
    private void startSpeech(JSONObject q) throws Exception {
        final long id=q.getLong("id");
        if (!pending.contains(id) || !foreground) { speechId=0; return; }
        JSONObject a=q.getJSONObject("args");
        if (!a.isNull("language") && tts.setLanguage(Locale.forLanguageTag(a.getString("language"))) < 0) {
            fail(id,"UnsupportedFeature","Speech language unavailable"); speechId=0; return;
        }
        if (!speechFocus.focus()) { fail(id,"ResourceBusy","Audio focus unavailable"); speechId=0; return; }
        tts.setSpeechRate((float)a.getDouble("rate"));
        tts.setOnUtteranceProgressListener(new UtteranceProgressListener() {
            @Override public void onStart(String utterance) { }
            @Override public void onDone(String utterance) { ui.post(() -> { done(id,null); speechId=0; speechFocus.abandonFocus(); }); }
            @Override public void onError(String utterance) { ui.post(() -> { fail(id,"DeviceIOError","Speech failed"); speechId=0; speechFocus.abandonFocus(); }); }
        });
        if (tts.speak(a.getString("text"),TextToSpeech.QUEUE_FLUSH,null,Long.toString(id)) != TextToSpeech.SUCCESS) {
            fail(id,"DeviceIOError","Speech failed"); speechId=0; speechFocus.abandonFocus();
        }
    }
    private void share(JSONObject q) throws Exception {
        long id=q.getLong("id"); JSONObject a=q.getJSONObject("args");
        io.execute(() -> {
            try {
                ArrayList<Uri> uris=new ArrayList<>();
                JSONArray sources=a.getJSONArray("uris");
                if (sources.length()>10) throw new IOException("Share at most 10 files");
                for (int i=0;i<sources.length();i++) {
                    Uri source=uri(sources.getString(i));
                    String name=describe(source).optString("name","file");
                    String extension=name.contains(".") ? name.substring(name.lastIndexOf('.')).replaceAll("[^.a-zA-Z0-9]","") : ".bin";
                    if (extension.length()>12) extension=".bin";
                    File target=File.createTempFile("share-",extension,SharedFiles.root(activity));
                    try (OutputStream out=new FileOutputStream(target)) { transfer(id,source,out,256L*1024*1024); }
                    catch (Exception e) { target.delete(); throw e; }
                    uris.add(SharedFiles.uri(activity,target));
                }
                ui.post(() -> {
                    if (!pending.contains(id)) return;
                    try {
                        Intent intent=new Intent(uris.size()>1 ? Intent.ACTION_SEND_MULTIPLE : Intent.ACTION_SEND).setType(a.optString("mime","*/*"));
                        if (uris.size()==1) intent.putExtra(Intent.EXTRA_STREAM,uris.get(0));
                        else if (!uris.isEmpty()) intent.putParcelableArrayListExtra(Intent.EXTRA_STREAM,uris);
                        intent.putExtra(Intent.EXTRA_TEXT,a.optString("text"));
                        if (!uris.isEmpty()) {
                            ClipData clip=ClipData.newRawUri("files",uris.get(0));
                            for (int i=1;i<uris.size();i++) clip.addItem(new ClipData.Item(uris.get(i)));
                            intent.setClipData(clip); intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
                        }
                        activity.startActivity(Intent.createChooser(intent,"Share")); done(id,null);
                    } catch (Exception e) { failure(id,e); }
                });
            } catch (Exception e) { failure(id,e); }
        });
    }
    private static int sensorType(String name) {
        switch(name) {
            case "accelerometer": case "shake": return Sensor.TYPE_ACCELEROMETER;
            case "linear_acceleration": return Sensor.TYPE_LINEAR_ACCELERATION;
            case "gyroscope": return Sensor.TYPE_GYROSCOPE;
            case "rotation": return Sensor.TYPE_ROTATION_VECTOR;
            case "light": return Sensor.TYPE_LIGHT;
            case "proximity": return Sensor.TYPE_PROXIMITY;
            case "magnetometer": return Sensor.TYPE_MAGNETIC_FIELD;
            default: return -100;
        }
    }
    private final class SensorWatch implements SensorEventListener {
        long requestId;
        final long handle;
        final SensorManager manager;
        final Sensor sensor;
        final String name;
        final int period;
        long last=0, lastShake=0, dropped=0;
        double gravityX=0,gravityY=0,gravityZ=9.81;
        SensorWatch(long handle,String name,double hz) {
            this.handle=handle; this.name=name;
            period=(int)(1000000/Math.max(1,Math.min(100,hz)));
            manager=(SensorManager)activity.getSystemService(Context.SENSOR_SERVICE);
            sensor=manager.getDefaultSensor(sensorType(name));
        }
        void start() {
            if (!manager.registerListener(this,sensor,period,ui)) throw new IllegalStateException("Sensor registration failed");
        }
        void stop() { manager.unregisterListener(this); samples.remove(handle); }
        @Override public void onAccuracyChanged(Sensor sensor,int accuracy) { }
        @Override public void onSensorChanged(SensorEvent event) {
            if (!foreground || event.timestamp-last < period*1000L) return;
            last=event.timestamp;
            if (name.equals("shake")) {
                gravityX=.8*gravityX+.2*event.values[0]; gravityY=.8*gravityY+.2*event.values[1]; gravityZ=.8*gravityZ+.2*event.values[2];
                double x=event.values[0]-gravityX,y=event.values[1]-gravityY,z=event.values[2]-gravityZ;
                if (x*x+y*y+z*z < 144 || event.timestamp-lastShake < 800000000L) return;
                lastShake=event.timestamp;
            }
            JSONArray values=new JSONArray();
            for (float v:event.values) { if (!Float.isFinite(v)) return; values.put(Float.valueOf(v)); }
            if (samples.containsKey(handle)) dropped++;
            samples.put(handle,obj("subscription",handle,"value",obj("values",values,"timestamp_ns",event.timestamp,"accuracy",event.accuracy,"dropped",dropped)));
        }
    }
    private void unsubscribe(long id) { SensorWatch watch=watches.remove(id); if(watch!=null) watch.stop(); }
    private void cancel(long id) {
        stopLocation(id);
        for(Sound sound:new ArrayList<>(sounds.values())) if(sound.requestId==id) { sound.close(); sounds.remove(sound.handle); }
        for(SensorWatch watch:new ArrayList<>(watches.values())) if(watch.requestId==id) unsubscribe(watch.handle);
        CancellationSignal signal=authentication.remove(id); if(signal!=null) signal.cancel();
        if (speechId==id) { if(tts!=null) tts.stop(); speechId=0; }
        // Keep dialog routing occupied until its result arrives. Never misroute a late result.
    }
    public void resume() {
        foreground=true;
        if(resumeRequest!=null) { JSONObject request=resumeRequest; resumeRequest=null; if(pending.contains(request.optLong("id"))) execute(request); }
        for(SensorWatch watch:watches.values()) watch.start();
    }
    public void pause() {
        foreground=false;
        for(Long id:new ArrayList<>(locations.keySet())) { stopLocation(id); fail(id,"OperationInterrupted","Location stopped in background"); }
        for(SensorWatch watch:watches.values()) watch.stop();
        for(Sound sound:sounds.values()) {
            sound.finishRecording("background");
            if(sound.player!=null) try { if(sound.player.isPlaying()) sound.player.pause(); sound.state="paused"; sound.reason="background"; sound.abandonFocus(); } catch(Exception ignored) { }
        }
        if(tts!=null) { tts.stop(); if(speechId!=0) fail(speechId,"OperationInterrupted","Speech stopped in background"); speechId=0; }
        if(torchId!=null) try { ((CameraManager)activity.getSystemService(Context.CAMERA_SERVICE)).setTorchMode(torchId,false); } catch(Exception ignored) { }
    }
    private void reset() {
        for(Long id:new ArrayList<>(locations.keySet())) stopLocation(id);
        for(CancellationSignal signal:authentication.values()) signal.cancel(); authentication.clear();
        for(SensorWatch watch:watches.values()) watch.stop(); watches.clear(); samples.clear();
        for(Sound sound:sounds.values()) sound.close(); sounds.clear();
        if(tts!=null) tts.stop(); speechId=0;
        speechFocus.abandonFocus();
    }
    public void destroy() {
        pause(); reset(); destroyed=true; io.shutdownNow();
        activity.unregisterReceiver(noisy);
        if(tts!=null) { tts.shutdown(); tts=null; }
        if(captureFile!=null) captureFile.delete();
        if(current==this) current=null;
    }
}
