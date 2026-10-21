# v0.9: Python code calling `v09_escapes`'s module-level lambda alias
# (`tests/v09_native_state.rs` imports the fixture as `m`, compiled or not).
import m

print(m.ESC("<a href='x'>"), m.ESC(u="&"), m.esc_all(["<"]))
