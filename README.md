# uwu - ujhhgtg's whiteboard, unleashed

a high-performance whiteboard app written in rust

reinventing the wheel because ~~others suck~~ why not

## features

- lightning-fast startup speed

- good frame rates

- multi-touch support for every tool

- overlay mode with 100% feature parity with standard mode

- first-class linux support

- awesome name

## building

### prepare

```bash
rustup toolchain install nightly
rustup default nightly

# --- system deps ---
sudo apt install libasound2-dev libglib2.0-dev libgtk-3-dev libappindicator3-dev libxdo-dev pkg-config
# or for arch linux
yay -S alsa-lib gtk3 libappindicator xdotool pkgconf
# --- end ---
```

### compile

```bash
cargo build --release
# or with cjk font embedded
cargo build --release --no-default-features --features embedded_font
# or with profiling
cargo build --release --no-default-features --features embedded_font,profiling
```

### cross-compiling for windows from linux

#### prepare

good luck figuring this out if you're not using arch (download & install manually from [gh releases](https://github.com/mstorsjo/llvm-mingw/releases))

```bash
# first add chaotic-aur, then
yay -S llvm-mingw llvm lld
```

add the following to `~/.cargo/config.toml`

```toml
[target.x86_64-pc-windows-gnullvm]
linker = "x86_64-w64-mingw32-clang"
ar = "x86_64-w64-mingw32-ar"
rustflags = ["-Ctarget-feature=+crt-static"] # statically link libunwind.dll

[target.aarch64-pc-windows-gnullvm]
linker = "aarch64-w64-mingw32-clang"
ar = "aarch64-w64-mingw32-ar"
rustflags = ["-Ctarget-feature=+crt-static"] # statically link libunwind.dll
```

#### compile x86_64

```bash
export PATH=/opt/llvm-mingw/bin/:$PATH
rustup target add x86_64-pc-windows-gnullvm
cargo build --release --target x86_64-pc-windows-gnullvm
```

#### compile aarch64

```bash
export PATH=/opt/llvm-mingw/bin/:$PATH
rustup target add aarch64-pc-windows-gnullvm
cargo build --release --target aarch64-pc-windows-gnullvm
```

## tech stack

egui + wgpu + winit

## supported platforms

- windows

- macos (untested)

- linux (mouse passthrough support might vary from DE/WMs; tested: GNOME (mutter), KDE (KWin), Hyprland)

## credits

[noto cjk](https://github.com/notofonts/noto-cjk)

[maple mono](https://github.com/subframe7536/Maple-font)

## license

gpl 3
