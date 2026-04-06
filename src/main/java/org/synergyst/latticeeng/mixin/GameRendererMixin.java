package org.synergyst.latticeeng.mixin;

import org.synergyst.latticeeng.RenderInterop;
import net.minecraft.client.render.GameRenderer;
import net.minecraft.client.render.RenderTickCounter;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Injects at the tail of GameRenderer.render() — the frame is complete
 * in MC's main framebuffer but not yet presented.
 *
 * NOTE: Verify the method descriptor against yarn mappings for your
 * exact MC version.  For 1.21.1 yarn, the signature is
 *   render(Lnet/minecraft/client/render/RenderTickCounter;Z)V
 */
@Mixin(GameRenderer.class)
public abstract class GameRendererMixin {

    @Inject(method = "render", at = @At("TAIL"))
    private void lattice$afterRender(RenderTickCounter tickCounter, boolean tick, CallbackInfo ci) {
        RenderInterop.onFrameRendered();
    }
}