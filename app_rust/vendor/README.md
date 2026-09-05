# Compatibility Patches

These are copies of the published crates.io packages, selected with
`[patch.crates-io]` in `../Cargo.toml`. Keep the upstream versions, renderer
dependencies, features, and license notices intact. Do not edit the Cargo
registry cache to apply these fixes.

- `nfd-0.0.4` (MIT): remove the trailing semicolon from the `nfd!` macro's
  expression in `build.rs` (`semicolon_in_expressions_from_macros`).
- `wgpu-core-0.7.2` (MPL-2.0): put the `Error` derive before its `#[error(...)]`
  helper attribute on `InvalidDevice` and `InvalidAdapter`
  (`legacy_derive_helpers`).

Path dependencies expose additional ordinary warnings that Cargo normally caps
for registry dependencies. The local copies also include explicit lifetimes,
unqualified imported names, declarations of existing renderer cfg aliases, and
documented dead-code allowances on retained upstream renderer types/fields.
Declaring the historical `tracing` cfg value does not enable that feature.

The `nfd` build script uses `cc` (already used elsewhere in the dependency tree)
instead of its deprecated predecessor `gcc`. Its Rust code uses `?` and
`Error::source` instead of deprecated APIs, with a no-dialog error-path test.
Windows builds explicitly link `shell32` for `SHCreateItemFromParsingName`
instead of relying on the application to supply that library indirectly.
The native file-picker sources and application-facing dialog API are unchanged.

These source-only corrections retain the existing file picker and Iced 0.3
renderer APIs. Remove the overrides when upgrading to upstream versions that
include the fixes. Do not silence Cargo's future-incompatibility reports.

Upstream package archives:

- <https://static.crates.io/crates/nfd/nfd-0.0.4.crate>
- <https://static.crates.io/crates/wgpu-core/wgpu-core-0.7.2.crate>

The `wgpu-core` source and modifications remain covered by MPL-2.0; see
`wgpu-core-0.7.2/LICENSE.md`. Include these sources and notices when distributing
the patched renderer, as required by its license.
