package org.synergyst.latticeeng.client;

import net.fabricmc.api.ClientModInitializer;
import org.synergyst.latticeeng.LatticeEng;
import org.synergyst.latticeeng.NativeBridge;
import org.synergyst.latticeeng.RenderInterop;

/**
 * Client entry point — loads native lib, sets up GL↔VK interop.
 */
public class LatticeEngClient implements ClientModInitializer {

    @Override
    public void onInitializeClient() {
        LatticeEng.LOG.info("[LatticeEng] Client init — loading native library");

        try {
            NativeBridge.loadNative();
            LatticeEng.LOG.info("[LatticeEng] Native library loaded");
        } catch (Exception e) {
            LatticeEng.LOG.error("[LatticeEng] Failed to load native — mod inactive", e);
            RenderInterop.disable();
        }

        Runtime.getRuntime().addShutdownHook(new Thread(RenderInterop::shutdown));
    }
}