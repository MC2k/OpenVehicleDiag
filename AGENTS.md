# OpenVehicleDiag Agent Guide

## Project Overview

OpenVehicleDiag is a Rust desktop application for vehicle diagnostics. The UI
uses Iced and the application supports SAE J2534 adapters plus a Windows SLCAN
serial-CAN transport.

- `app_rust/`: Rust application. Run all Rust commands from this directory.
- `app_rust/src/commapi/`: Adapter abstractions and implementations.
- `app_rust/src/commapi/slcan_api.rs`: Windows SLCAN transport and host-side
  ISO-TP implementation for CAN, OBD-II, UDS, and KWP2000 traffic.
- `app_rust/src/windows/launcher.rs`: Adapter/API selection UI.
- `app_rust/src/windows/cantracer.rs`: Raw CAN Analyzer UI.
- `SLCAN_PROTOCOL_SPEC.md`: SLCAN wire contract for the supported adapter.
- `common/`: Shared diagnostic schema library.
- `app_rust/vendor/README.md`: Local compatibility patches for the existing
  file-picker and renderer dependencies; preserve these overrides and notices.

## Working Rules

- Preserve existing adapter behavior unless the requested change requires it.
- SLCAN is classic CAN only. ISO-TP is implemented on the host; extended
  ISO-TP addressing and payloads over 4095 bytes are unsupported.
- Keep changes small and validate protocol behavior against
  `SLCAN_PROTOCOL_SPEC.md`.
- Do not modify generated build artifacts under `app_rust/target/`.
- Keep Windows/MSVC builds warning-free. Do not silence future-incompatibility
  reporting or modify the Cargo registry cache to work around dependency issues.

## Build And Test Handoff

Delegate all compilation, tests, linting, and build-result review to the
`@build-review` subagent. It uses `openai/gpt-5.6-luna`, knows the project
build requirements, and cannot edit source files.

Ask it to run the relevant command from `app_rust`, then use its concise result
to decide whether implementation changes are needed. Do not run builds or
tests in the primary implementation session unless the build-review agent is
unavailable.
