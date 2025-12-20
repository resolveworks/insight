# Tauri + SvelteKit + TypeScript

This template should help get you started developing with Tauri, SvelteKit and TypeScript in Vite.

## Building (CPU and GPU)

- CPU default: `pnpm tauri build` (or `cargo tauri build` inside `src-tauri`) builds a portable CPU-only binary.
- NVIDIA GPU: enable CUDA + flash attention + cuDNN:  
  `pnpm tauri build -- --features "cuda flash-attn cudnn"`  
  (requires CUDA toolkit/cuDNN and matching drivers on the build machine)
- Apple Silicon GPU: use Metal:  
  `pnpm tauri build -- --features metal`
- Intel CPU acceleration: `pnpm tauri build -- --features mkl`
- Apple Accelerate (CPU): `pnpm tauri build -- --features accelerate`

If a feature isn’t available on the build machine, stick to the CPU default. GPU builds are optional; CPU builds still work for users without a GPU.

## Recommended IDE Setup

[VS Code](https://code.visualstudio.com/) + [Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer).
