package org.synergyst.latticeeng;

import net.minecraft.client.MinecraftClient;
import net.minecraft.client.gl.Framebuffer;
import net.minecraft.text.Text;
import org.lwjgl.opengl.*;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Manages the GL side of the GL↔VK shared textures and the per-frame blit loop.
 *
 * Flow each frame:
 *   MC fb → blit → inputTex (shared) → glFinish
 *   → native processFrame (VK compute) → vkQueueWaitIdle
 *   → outputTex (shared) → blit → MC fb
 */
public final class RenderInterop {

    private static final Logger LOG = LoggerFactory.getLogger("RTXMod");

    // GL_EXT_memory_object handle-type constants
    private static final int GL_HANDLE_TYPE_OPAQUE_FD_EXT    = 0x9586;
    private static final int GL_HANDLE_TYPE_OPAQUE_WIN32_EXT = 0x9587;

    private static boolean disabled     = false;
    private static boolean initialized  = false;
    private static boolean statusSent   = false;

    private static int width, height;

    // GL objects
    private static int inputMemObj,  outputMemObj;
    private static int inputTex,     outputTex;
    private static int inputFbo,     outputFbo;

    private RenderInterop() {}

    /** Mark the mod as permanently off (native load failed, etc.). */
    public static void disable() { disabled = true; }

    public static boolean isActive() { return initialized && !disabled; }

    // ------------------------------------------------------------------
    // Called every frame from the mixin
    // ------------------------------------------------------------------

    public static void onFrameRendered() {
        if (disabled) return;

        MinecraftClient mc = MinecraftClient.getInstance();
        if (mc.getWindow() == null) return;

        int fbW = mc.getWindow().getFramebufferWidth();
        int fbH = mc.getWindow().getFramebufferHeight();
        if (fbW <= 0 || fbH <= 0) return;

        // Lazy init on first render (GL context is guaranteed here)
        if (!initialized) {
            if (!tryInit(fbW, fbH)) {
                disabled = true;
                return;
            }
        }

        // Handle resize
        if (fbW != width || fbH != height) {
            if (!handleResize(fbW, fbH)) {
                disabled = true;
                return;
            }
        }

        // --- per-frame GL↔VK round-trip --------------------------------------
        Framebuffer mcFb = mc.getFramebuffer();
        int mcFbo = mcFb.fbo;

        // save GL state
        int prevRead = GL11.glGetInteger(GL30.GL_READ_FRAMEBUFFER_BINDING);
        int prevDraw = GL11.glGetInteger(GL30.GL_DRAW_FRAMEBUFFER_BINDING);

        // MC fb → input shared texture
        GL30.glBindFramebuffer(GL30.GL_READ_FRAMEBUFFER, mcFbo);
        GL30.glBindFramebuffer(GL30.GL_DRAW_FRAMEBUFFER, inputFbo);
        GL30.glBlitFramebuffer(0, 0, width, height, 0, 0, width, height,
                GL11.GL_COLOR_BUFFER_BIT, GL11.GL_NEAREST);

        GL11.glFinish(); // flush GL writes before VK reads

        // Vulkan compute
        if (!NativeBridge.processFrame()) {
            LOG.error("Native processFrame failed — disabling");
            disabled = true;
            restoreFbo(prevRead, prevDraw);
            return;
        }

        // output shared texture → MC fb
        GL30.glBindFramebuffer(GL30.GL_READ_FRAMEBUFFER, outputFbo);
        GL30.glBindFramebuffer(GL30.GL_DRAW_FRAMEBUFFER, mcFbo);
        GL30.glBlitFramebuffer(0, 0, width, height, 0, 0, width, height,
                GL11.GL_COLOR_BUFFER_BIT, GL11.GL_NEAREST);

        restoreFbo(prevRead, prevDraw);

        // One-time status message
        if (!statusSent && mc.player != null) {
            String gpu = NativeBridge.getGpuName();
            boolean rt = NativeBridge.isRtSupported();
            mc.player.sendMessage(
                    Text.literal("§6[RTX Mod]§r Active | GPU: §b" + gpu
                            + "§r | RT: " + (rt ? "§aYes" : "§cNo")),
                    false);
            statusSent = true;
        }
    }

    // ------------------------------------------------------------------
    // init / resize / cleanup helpers
    // ------------------------------------------------------------------

    private static boolean tryInit(int w, int h) {
        LOG.info("RenderInterop init {}×{}", w, h);

        // Check GL extensions
        if (!hasGLExtension("GL_EXT_memory_object")) {
            LOG.error("GL_EXT_memory_object not available");
            return false;
        }
        boolean linux = isLinux();
        if (linux && !hasGLExtension("GL_EXT_memory_object_fd")) {
            LOG.error("GL_EXT_memory_object_fd not available");
            return false;
        }
        if (!linux && !hasGLExtension("GL_EXT_memory_object_win32")) {
            LOG.error("GL_EXT_memory_object_win32 not available");
            return false;
        }

        long[] handles = NativeBridge.init(w, h);
        if (handles == null || handles.length < 4) {
            LOG.error("Native init returned null");
            return false;
        }

        width  = w;
        height = h;

        if (!createGlResources(handles)) return false;

        initialized = true;
        LOG.info("RenderInterop ready — GPU: {}, RT: {}",
                NativeBridge.getGpuName(), NativeBridge.isRtSupported());
        return true;
    }

    private static boolean handleResize(int w, int h) {
        LOG.info("Resize {}×{} → {}×{}", width, height, w, h);
        destroyGlResources();

        long[] handles = NativeBridge.resize(w, h);
        if (handles == null || handles.length < 4) {
            LOG.error("Native resize returned null");
            return false;
        }
        width  = w;
        height = h;
        return createGlResources(handles);
    }

    /** Import VK-exported memory into GL textures + FBOs. */
    private static boolean createGlResources(long[] handles) {
        long inHandle  = handles[0], inSize  = handles[1];
        long outHandle = handles[2], outSize = handles[3];

        try {
            inputMemObj  = EXTMemoryObject.glCreateMemoryObjectsEXT();
            outputMemObj = EXTMemoryObject.glCreateMemoryObjectsEXT();

            if (isLinux()) {
                EXTMemoryObjectFD.glImportMemoryFdEXT(
                        inputMemObj,  inSize,  GL_HANDLE_TYPE_OPAQUE_FD_EXT, (int) inHandle);
                EXTMemoryObjectFD.glImportMemoryFdEXT(
                        outputMemObj, outSize, GL_HANDLE_TYPE_OPAQUE_FD_EXT, (int) outHandle);
            } else {
                EXTMemoryObjectWin32.glImportMemoryWin32HandleEXT(
                        inputMemObj,  inSize,  GL_HANDLE_TYPE_OPAQUE_WIN32_EXT, inHandle);
                EXTMemoryObjectWin32.glImportMemoryWin32HandleEXT(
                        outputMemObj, outSize, GL_HANDLE_TYPE_OPAQUE_WIN32_EXT, outHandle);
            }

            inputTex  = GL11.glGenTextures();
            outputTex = GL11.glGenTextures();

            GL11.glBindTexture(GL11.GL_TEXTURE_2D, inputTex);
            EXTMemoryObject.glTexStorageMem2DEXT(
                    GL11.GL_TEXTURE_2D, 1, GL30.GL_RGBA8, width, height, inputMemObj, 0);

            GL11.glBindTexture(GL11.GL_TEXTURE_2D, outputTex);
            EXTMemoryObject.glTexStorageMem2DEXT(
                    GL11.GL_TEXTURE_2D, 1, GL30.GL_RGBA8, width, height, outputMemObj, 0);

            GL11.glBindTexture(GL11.GL_TEXTURE_2D, 0);

            // FBOs
            inputFbo = GL30.glGenFramebuffers();
            GL30.glBindFramebuffer(GL30.GL_FRAMEBUFFER, inputFbo);
            GL30.glFramebufferTexture2D(GL30.GL_FRAMEBUFFER,
                    GL30.GL_COLOR_ATTACHMENT0, GL11.GL_TEXTURE_2D, inputTex, 0);
            if (GL30.glCheckFramebufferStatus(GL30.GL_FRAMEBUFFER) != GL30.GL_FRAMEBUFFER_COMPLETE) {
                LOG.error("Input FBO incomplete");
                return false;
            }

            outputFbo = GL30.glGenFramebuffers();
            GL30.glBindFramebuffer(GL30.GL_FRAMEBUFFER, outputFbo);
            GL30.glFramebufferTexture2D(GL30.GL_FRAMEBUFFER,
                    GL30.GL_COLOR_ATTACHMENT0, GL11.GL_TEXTURE_2D, outputTex, 0);
            if (GL30.glCheckFramebufferStatus(GL30.GL_FRAMEBUFFER) != GL30.GL_FRAMEBUFFER_COMPLETE) {
                LOG.error("Output FBO incomplete");
                return false;
            }

            GL30.glBindFramebuffer(GL30.GL_FRAMEBUFFER, 0);
            return true;
        } catch (Exception e) {
            LOG.error("GL resource creation failed", e);
            return false;
        }
    }

    private static void destroyGlResources() {
        if (inputFbo  != 0) { GL30.glDeleteFramebuffers(inputFbo);  inputFbo  = 0; }
        if (outputFbo != 0) { GL30.glDeleteFramebuffers(outputFbo); outputFbo = 0; }
        if (inputTex  != 0) { GL11.glDeleteTextures(inputTex);      inputTex  = 0; }
        if (outputTex != 0) { GL11.glDeleteTextures(outputTex);     outputTex = 0; }
        // Memory objects are freed when their textures are deleted.
        // The imported fd/handle is consumed by the driver on import.
        inputMemObj = outputMemObj = 0;
    }

    /** Full shutdown (called from mod unload / JVM exit). */
    public static void shutdown() {
        if (initialized) {
            destroyGlResources();
            NativeBridge.shutdown();
            initialized = false;
        }
    }

    // ------------------------------------------------------------------
    // utilities
    // ------------------------------------------------------------------

    private static void restoreFbo(int read, int draw) {
        GL30.glBindFramebuffer(GL30.GL_READ_FRAMEBUFFER, read);
        GL30.glBindFramebuffer(GL30.GL_DRAW_FRAMEBUFFER, draw);
    }

    private static boolean isLinux() {
        return System.getProperty("os.name").toLowerCase().contains("linux");
    }

    private static boolean hasGLExtension(String name) {
        int n = GL11.glGetInteger(GL30.GL_NUM_EXTENSIONS);
        for (int i = 0; i < n; i++) {
            if (name.equals(GL30.glGetStringi(GL11.GL_EXTENSIONS, i))) return true;
        }
        return false;
    }
}