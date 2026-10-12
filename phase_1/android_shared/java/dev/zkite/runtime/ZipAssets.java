package dev.zkite.runtime;

import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.util.Enumeration;
import java.util.zip.CRC32;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

/** Installs managed runtime assets when their bytes in the installed APK change.
 *
 * App source, framework, fonts and stdlib are managed directories. Application data
 * belongs outside those directories and is never touched here. A changed archive
 * replaces its complete directory so removed Python modules cannot survive updates.
 * The stamp travels inside that directory, making content and identity one rename.
 * This class has no Android dependencies so the actual installer is host-testable.
 */
public final class ZipAssets {
    private static final String STAMP = ".zkite-asset-id";

    private ZipAssets() {}

    /** @return true if new content was installed, false for an unchanged asset. */
    public static synchronized boolean unpack(File apk, String assetName, File destination,
                                              String version) throws IOException {
        if (Files.isSymbolicLink(destination.toPath())) {
            throw new IOException("asset destination must not be a symlink: " + destination);
        }
        File dest = destination.getCanonicalFile();
        File parent = dest.getParentFile();
        if (parent == null) throw new IOException("asset destination has no parent: " + dest);
        makeDirectory(parent);
        File next = new File(parent, "." + dest.getName() + ".zkite-next");
        File old = new File(parent, "." + dest.getName() + ".zkite-old");
        File archive = new File(parent, "." + dest.getName() + ".zkite-asset.zip");

        // Recover an interrupted pair of renames before consulting the stamp. A
        // complete new directory wins; otherwise put the last complete one back.
        if (!dest.exists() && old.exists() && !old.renameTo(dest)) {
            throw new IOException("could not restore previous asset directory: " + dest);
        }
        deleteTree(old);
        deleteTree(next);
        deleteTree(archive);

        try (ZipFile installed = new ZipFile(apk)) {
            ZipEntry asset = installed.getEntry("assets/" + assetName);
            if (asset == null || asset.isDirectory()) {
                throw new IOException("APK is missing asset: " + assetName);
            }
            String identity = "zkite-assets-v1:" + version + ":" + assetName + ":"
                    + asset.getCrc() + ":" + asset.getSize();
            if (dest.isDirectory() && identity.equals(readStamp(dest))) return false;

            try {
                // ZipFile validates the nested ZIP's central directory as well as
                // its local records. ZipInputStream alone accepts truncated endings.
                try (InputStream in = installed.getInputStream(asset);
                     FileOutputStream out = new FileOutputStream(archive)) {
                    copyVerified(in, out, asset);
                }
                makeDirectory(next);
                extract(archive, next);
                try (DataOutputStream stamp = new DataOutputStream(
                        new FileOutputStream(new File(next, STAMP)))) {
                    stamp.writeUTF(identity);
                }

                if (dest.exists() && !dest.renameTo(old)) {
                    throw new IOException("could not preserve previous asset directory: " + dest);
                }
                if (!next.renameTo(dest)) {
                    throw new IOException("could not install asset directory: " + dest);
                }
                deleteTree(old);
                return true;
            } catch (IOException failure) {
                if (!dest.exists() && old.exists() && !old.renameTo(dest)) {
                    failure.addSuppressed(new IOException("previous asset remains at " + old));
                }
                throw failure;
            } finally {
                deleteTree(next);
                deleteTree(archive);
            }
        }
    }

    private static String readStamp(File directory) {
        try (DataInputStream in = new DataInputStream(new FileInputStream(new File(directory, STAMP)))) {
            return in.readUTF();
        } catch (IOException ignored) {
            return null; // also migrates the old, empty, version-only stamps
        }
    }

    private static void extract(File archive, File dest) throws IOException {
        String prefix = dest.getCanonicalPath() + File.separator;
        try (ZipFile zip = new ZipFile(archive)) {
            Enumeration<? extends ZipEntry> entries = zip.entries();
            while (entries.hasMoreElements()) {
                ZipEntry entry = entries.nextElement();
                String name = entry.getName();
                File output = new File(dest, name).getCanonicalFile();
                if (!output.getPath().startsWith(prefix) || name.indexOf('\\') >= 0
                        || name.equals(STAMP)) {
                    throw new IOException("zip entry outside target or reserved: " + name);
                }
                if (entry.isDirectory()) {
                    makeDirectory(output);
                    continue;
                }
                makeDirectory(output.getParentFile());
                try (InputStream in = zip.getInputStream(entry);
                     FileOutputStream out = new FileOutputStream(output)) {
                    copyVerified(in, out, entry);
                }
            }
        }
    }

    private static void copyVerified(InputStream in, FileOutputStream out, ZipEntry entry)
            throws IOException {
        byte[] buffer = new byte[64 * 1024];
        CRC32 crc = new CRC32();
        long size = 0;
        int count;
        while ((count = in.read(buffer)) != -1) {
            out.write(buffer, 0, count);
            crc.update(buffer, 0, count);
            size += count;
        }
        if (size != entry.getSize() || crc.getValue() != entry.getCrc()) {
            throw new IOException("corrupt zip entry: " + entry.getName());
        }
    }

    private static void makeDirectory(File directory) throws IOException {
        if (!directory.isDirectory() && !directory.mkdirs()) {
            throw new IOException("could not create " + directory);
        }
    }

    private static void deleteTree(File file) throws IOException {
        // Never follow a symlink when cleaning a managed extraction directory.
        if (file.isDirectory() && !Files.isSymbolicLink(file.toPath())) {
            File[] children = file.listFiles();
            if (children == null) throw new IOException("could not list " + file);
            for (File child : children) deleteTree(child);
        }
        if ((file.exists() || Files.isSymbolicLink(file.toPath())) && !file.delete()) {
            throw new IOException("could not remove " + file);
        }
    }
}
