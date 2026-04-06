package org.synergyst.latticeeng;

import net.fabricmc.api.ModInitializer;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Common entry point — runs on both client and server.
 * Currently: just logging + future config placeholder.
 * All rendering logic lives in LatticeEngClient (ClientModInitializer).
 */
public class LatticeEng implements ModInitializer {

    public static final String MOD_ID = "latticeeng";
    public static final Logger LOG = LoggerFactory.getLogger(MOD_ID);

    @Override
    public void onInitialize() {
        LOG.info("[LatticeEng] Common init");

        // TODO: register config (e.g. cloth-config / YACL)
        // TODO: register network packets for settings sync
    }
}