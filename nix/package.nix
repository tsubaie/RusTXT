# RusTXT for Nix: `nix build`, `nix run`, or add this flake's overlay to a NixOS config.
{
  lib,
  rustPlatform,
  pkg-config,
  fontconfig,
  wayland,
  libxkbcommon,
  vulkan-loader,
  libGL,
  libx11,
  libxcursor,
  libxi,
  libxrandr,
  libxcb,
}:
rustPlatform.buildRustPackage {
  pname = "rustxt";
  version = (lib.importTOML ../Cargo.toml).workspace.package.version or "0";

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.difference ../. (
      lib.fileset.unions [
        ../nix
        ../flake.nix
        (lib.fileset.maybeMissing ../flake.lock)
        ../docs
      ]
    );
  };
  cargoLock.lockFile = ../Cargo.lock;

  # Only the desktop app (the workspace also has rustxt-core)
  cargoBuildFlags = [ "-p" "rustxt-iced" ];
  cargoTestFlags = [ "-p" "rustxt-iced" ];

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ fontconfig ];

  # Same layout as the Arch package: binary "rustxt", desktop entry, icon
  postInstall = ''
    if [ -e $out/bin/rustxt-iced ]; then mv $out/bin/rustxt-iced $out/bin/rustxt; fi
    install -Dm644 data/com.tsubaie.rustxt.desktop $out/share/applications/com.tsubaie.rustxt.desktop
    install -Dm644 data/icons/com.tsubaie.rustxt.png $out/share/icons/hicolor/512x512/apps/com.tsubaie.rustxt.png
  '';

  # winit/wgpu load these with dlopen() at runtime, which Nix can't see
  postFixup = ''
    patchelf --add-rpath ${
      lib.makeLibraryPath [
        wayland
        libxkbcommon
        vulkan-loader
        libGL
        fontconfig
        libx11
        libxcursor
        libxi
        libxrandr
        libxcb
      ]
    } $out/bin/rustxt
  '';

  meta = {
    description = "Dead simple, recoverable text editor built with Rust and Iced";
    homepage = "https://github.com/tsubaie/RusTXT";
    license = lib.licenses.mit;
    mainProgram = "rustxt";
    platforms = lib.platforms.linux;
  };
}
