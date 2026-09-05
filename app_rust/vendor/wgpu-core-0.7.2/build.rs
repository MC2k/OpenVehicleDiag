/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

fn main() {
    // Declare every alias, including those disabled for the current target.
    println!("cargo:rustc-check-cfg=cfg(wasm)");
    println!("cargo:rustc-check-cfg=cfg(apple)");
    println!("cargo:rustc-check-cfg=cfg(unix_wo_apple)");
    println!("cargo:rustc-check-cfg=cfg(vulkan)");
    println!("cargo:rustc-check-cfg=cfg(metal)");
    println!("cargo:rustc-check-cfg=cfg(dx12)");
    println!("cargo:rustc-check-cfg=cfg(dx11)");
    println!("cargo:rustc-check-cfg=cfg(gl)");
    // Accept the historical allocator instrumentation cfg without enabling it.
    println!("cargo:rustc-check-cfg=cfg(feature, values(\"tracing\"))");

    // Setup cfg aliases
    cfg_aliases::cfg_aliases! {
        // Vendors/systems
        wasm: { target_arch = "wasm32" },
        apple: { any(target_os = "ios", target_os = "macos") },
        unix_wo_apple: {all(unix, not(apple))},

        // Backends
        vulkan: { all(not(wasm), any(windows, unix_wo_apple, feature = "gfx-backend-vulkan")) },
        metal: { all(not(wasm), apple) },
        dx12: { all(not(wasm), windows) },
        dx11: { all(not(wasm), windows) },
        gl: { any(wasm, unix_wo_apple) },
    }
}
