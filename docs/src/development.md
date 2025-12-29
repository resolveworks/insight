# Development

## Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) 20+
- [pnpm](https://pnpm.io/)
- Platform-specific dependencies (see below)

### Linux

```bash
sudo apt-get install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf
```

### macOS

Xcode Command Line Tools:

```bash
xcode-select --install
```

### Windows

Visual Studio Build Tools with C++ workload.

## Running Locally

```bash
# Install frontend dependencies
pnpm install

# Start development mode
pnpm tauri dev
```

## Building

```bash
# Development build
pnpm tauri build --debug

# Release build
pnpm tauri build
```

### GPU Acceleration

```bash
# NVIDIA (CUDA)
cd src-tauri && cargo build --release --features "cuda flash-attn cudnn"

# Apple Silicon (Metal)
cd src-tauri && cargo build --release --features metal
```

## Testing

### Backend (Rust)

```bash
cd src-tauri && cargo test
```

### Frontend (Svelte)

```bash
pnpm test        # Watch mode
pnpm test:run    # Single run (CI)
```

## Project Structure

```
insight/
├── src/                 # Svelte frontend
├── src-tauri/          # Tauri + Rust backend
│   └── src/
│       ├── commands/   # Tauri commands (IPC)
│       └── ...
├── crates/
│   └── insight-core/   # Core library
└── docs/               # This documentation
```

## Understanding Dependencies

Prefer local tools over web searches:

```bash
# Generate and browse docs for exact dependency versions
cargo doc --open

# View dependency graph
cargo tree

# Source code at
~/.cargo/registry/src/
```
