# egui-desktop local patch

This is the `egui-desktop` 0.2.5 source with a local fix for nested submenu hit
testing. The upstream outside-click check considered only the first popup, so
clicking any cascading submenu was treated as a click outside and closed the
menu. The patched renderer tracks every open cascade rectangle.

`Cargo.toml` applies this copy through `[patch.crates-io]`.
