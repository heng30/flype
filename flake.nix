{
  description = "flype：Wayland 下的 fcitx5 拼音输入法";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-stable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: rec {
        flype = pkgs.callPackage ./nix/package.nix { withSettings = true; };
        default = flype;
      });

      overlays.default = final: prev: {
        flype = final.callPackage ./nix/package.nix { withSettings = true; };
      };
    };
}
