import dev.zkite.runtime.ZipAssets;
import java.io.File;

/** Calls the exact installer compiled into each Android shell. */
public final class ZipAssetsProbe {
    public static void main(String[] args) throws Exception {
        System.out.println(ZipAssets.unpack(new File(args[0]), args[1], new File(args[2]), args[3]));
    }
}
