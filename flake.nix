{
  description = "RusTXT: dead simple, recoverable text editor";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      forAllSystems = nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
      ];
    in
    {
      packages = forAllSystems (system: rec {
        rustxt = nixpkgs.legacyPackages.${system}.callPackage ./nix/package.nix { };
        default = rustxt;
      });

      # For NixOS configs: nixpkgs.overlays = [ inputs.rustxt.overlays.default ]; then pkgs.rustxt
      overlays.default = final: _prev: { rustxt = final.callPackage ./nix/package.nix { }; };
    };
}
