# Release Pipeline

This document describes the release pipeline for havy-os.

## Overview

The release pipeline uses [Release Please](https://github.com/googleapis/release-please) to automate releases based on [Conventional Commits](https://www.conventionalcommits.org/).

## Workflow

1. **Conventional Commits**: Push commits to `main` following conventional commit format:
   - `feat:` - triggers a minor version bump (0.1.x -> 0.2.0)
   - `fix:` - triggers a patch version bump (0.1.0 -> 0.1.1)
   - `feat!:` or `fix!:` - triggers a major version bump (breaking change)

2. **Release PR**: Release Please automatically creates/updates a release PR with:
   - Updated version in `Cargo.toml` (workspace)
   - Updated `CHANGELOG.md`
   - Both `kernel` and `mkfs` package versions

3. **Merge Release PR**: When you merge the release PR:
   - A GitHub release is created
   - The `build-and-upload` job runs automatically
   - Both `kernel` and `fs.img` are built and uploaded as release assets

## Release Assets

Every release includes two artifacts (uncompressed):
- `kernel` - The RISC-V kernel binary
- `fs.img` - The filesystem image containing all WASM binaries

## Build Process

The CI workflow follows these steps:
1. Setup Rust with targets: `riscv64gc-unknown-none-elf`, `wasm32-unknown-unknown`
2. Install `wasm-opt` for WASM optimization
3. Build the kernel for RISC-V target
4. Build WASM binaries with specific flags
5. Optimize WASM binaries
6. Generate the filesystem image
7. Upload both `kernel` and `fs.img` to the GitHub release

## Configuration Files

- `release-please-config.json` - Release Please configuration
- `.release-please-manifest.json` - Version tracking
- `.github/workflows/release-trigger.yml` - Main release workflow
- `Cargo.toml` - Workspace version definition
- `kernel/Cargo.toml` - Kernel package (inherits workspace version)
- `mkfs/Cargo.toml` - Filesystem tool package (inherits workspace version)

## Triggering a Release

Since both packages share the same version, any change to either `kernel` or `mkfs` that follows conventional commits will trigger a release containing both artifacts.

Example workflow:
```bash
# Make changes to kernel or mkfs
git add .
git commit -m "feat: add new feature"
git push origin main

# Release Please creates/updates a release PR
# Review and merge the PR
# Release is automatically created with kernel + fs.img
```

## Version Management

All packages in the workspace share the same version. The root `Cargo.toml` contains both a virtual package and workspace package definition:

```toml
[package]
name = "havy-os"
version = "0.1.12"  # Updated by Release Please
edition = "2021"
publish = false

[workspace.package]
version = "0.1.12"  # Also updated by Release Please
```

Both `kernel` and `mkfs` inherit the workspace version:

```toml
[package]
version.workspace = true
```

Release Please updates both version fields in the root Cargo.toml, ensuring all components stay synchronized.

