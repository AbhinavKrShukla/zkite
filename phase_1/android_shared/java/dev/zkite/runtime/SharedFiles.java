package dev.zkite.runtime;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import android.webkit.MimeTypeMap;
import java.io.File;
import java.io.FileNotFoundException;

/** Only individually granted files in cache/shared are externally accessible. */
public final class SharedFiles extends ContentProvider {
    public static File root(Context context) {
        File root = new File(context.getCacheDir(), "shared");
        root.mkdirs();
        return root;
    }
    public static Uri uri(Context context, File file) {
        return new Uri.Builder().scheme("content").authority(context.getPackageName() + ".zkite.files")
            .appendPath(file.getName()).build();
    }
    private File file(Uri uri) throws FileNotFoundException {
        try {
            File root = root(getContext()).getCanonicalFile();
            File result = new File(root, uri.getLastPathSegment()).getCanonicalFile();
            if (!root.equals(result.getParentFile())) throw new FileNotFoundException("Invalid shared path");
            return result;
        } catch (Exception e) { throw new FileNotFoundException("Invalid shared path"); }
    }
    @Override public boolean onCreate() { return true; }
    @Override public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        return ParcelFileDescriptor.open(file(uri), ParcelFileDescriptor.parseMode(mode));
    }
    @Override public String getType(Uri uri) {
        String name = uri.getLastPathSegment();
        String ext = name != null && name.contains(".") ? name.substring(name.lastIndexOf('.') + 1) : "";
        String type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext.toLowerCase(java.util.Locale.ROOT));
        return type == null ? "application/octet-stream" : type;
    }
    @Override public Cursor query(Uri uri, String[] projection, String selection, String[] args, String order) {
        String[] columns = projection == null ? new String[]{OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE} : projection;
        MatrixCursor cursor = new MatrixCursor(columns);
        try {
            File f = file(uri);
            Object[] row = new Object[columns.length];
            for (int i=0; i<columns.length; i++) {
                if (OpenableColumns.DISPLAY_NAME.equals(columns[i])) row[i] = f.getName();
                if (OpenableColumns.SIZE.equals(columns[i])) row[i] = f.length();
            }
            cursor.addRow(row);
        } catch (Exception ignored) { }
        return cursor;
    }
    @Override public Uri insert(Uri u, ContentValues v) { throw new UnsupportedOperationException(); }
    @Override public int delete(Uri u, String s, String[] a) { throw new UnsupportedOperationException(); }
    @Override public int update(Uri u, ContentValues v, String s, String[] a) { throw new UnsupportedOperationException(); }
}
