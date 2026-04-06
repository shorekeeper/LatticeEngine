package org.synergyst.latticeeng;

import java.io.*;
import java.nio.file.*;

/**
 * Thin JNI bridge to the Rust cdylib (rtx_native).
 * All methods are static and thread-safe on the native side (global Mutex).
 */
public final class NativeBridge {

    private static boolean loaded = false;

    private NativeBridge() {}

    /** Load the platform-specific native library. Idempotent. */
    public static synchronized void loadNative() {
        if (loaded) return;

        // Allow override for development: -Drtx.native.path=/abs/path/to/lib
        String custom = System.getProperty("rtx.native.path");
        if (custom != null) {
            System.load(custom);
            loaded = true;
            return;
        }

        String os = System.getProperty("os.name").toLowerCase();
        String libName;
        if (os.contains("win")) {
            libName = "lattice.dll";
        } else if (os.contains("linux")) {
            libName = "liblattice_native.so";
        } else {
            throw new UnsupportedOperationException("Unsupported OS: " + os);
        }

        try (InputStream in = NativeBridge.class.getResourceAsStream("/natives/" + libName)) {
            if (in == null)
                throw new FileNotFoundException("/natives/" + libName + " not found in JAR");
            Path tmp = Files.createTempDirectory("lattice");
            Path lib = tmp.resolve(libName);
            Files.copy(in, lib, StandardCopyOption.REPLACE_EXISTING);
            lib.toFile().deleteOnExit();
            tmp.toFile().deleteOnExit();
            System.load(lib.toAbsolutePath().toString());
            loaded = true;
        } catch (IOException e) {
            throw new RuntimeException("Failed to extract native library", e);
        }
    }

    /**
     * Init Vulkan, create shared images.
     * @return [in_handle, in_allocSize, out_handle, out_allocSize] or null.
     */
    public static native long[] init(int width, int height);

    /** Run one compute pass. Returns false on error. */
    public static native boolean processFrame();

    /** Recreate images at new size. Returns new handles or null. */
    public static native long[] resize(int width, int height);

    /** Full teardown. */
    public static native void shutdown();

    public static native String getGpuName();
    public static native boolean isRtSupported();
}