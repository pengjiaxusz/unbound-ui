#version 460

// ---------------------------------------------------------------------------
// Opaque Core pre-pass fragment shader (docs §3.1).
//
// This pass exists only to write depth for large opaque containers so that the
// hardware Early-Z unit can reject the thousands of primitives behind them. It
// intentionally produces no colour output: the render target has zero colour
// attachments and the shader has no outputs, so the whole pass is pure depth.
// ---------------------------------------------------------------------------

void main() {}
