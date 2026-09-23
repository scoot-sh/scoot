//! [`keeps_layout`](super::keeps_layout), the rule the scanout exporter
//! applies to every client buffer it turns into a framebuffer. The exporter
//! itself needs a DRM device and a client buffer GBM misreports, which no
//! machine here produces; the decision is what can be pinned, and it is the
//! whole of what the wrapper adds.

use smithay::backend::allocator::Modifier;

use super::keeps_layout;

const TILED: Modifier = Modifier::I915_x_tiled;
const OTHER_TILED: Modifier = Modifier::I915_y_tiled;

#[test]
fn a_tiled_client_buffer_needs_its_own_modifier_on_the_framebuffer() {
    assert!(keeps_layout(Some(TILED), TILED), "kept: scan it out");
    assert!(
        !keeps_layout(Some(TILED), Modifier::Invalid),
        "added without a modifier: KMS would read tiles as the implicit layout"
    );
    assert!(
        !keeps_layout(Some(TILED), Modifier::Linear),
        "added as LINEAR: the same scrambled picture"
    );
    assert!(
        !keeps_layout(Some(TILED), OTHER_TILED),
        "a different tiling is still the wrong one"
    );
}

#[test]
fn linear_and_implicit_client_buffers_pass_as_they_always_have() {
    // A single-plane LINEAR buffer at offset 0 is GBM-imported without
    // modifiers and so always comes back `Invalid` -- the path every
    // primary-direct frame seen so far has taken (virtio). Refusing it would
    // end primary-direct outright.
    assert!(keeps_layout(Some(Modifier::Linear), Modifier::Invalid));
    assert!(keeps_layout(Some(Modifier::Linear), Modifier::Linear));
    assert!(keeps_layout(Some(Modifier::Invalid), Modifier::Invalid));
    // Not a client dma-buf (a swapchain slot).
    assert!(keeps_layout(None, TILED));
    assert!(keeps_layout(None, Modifier::Invalid));
}
