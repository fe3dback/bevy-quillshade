{
  description = "bevy dev shell (multi-system)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config = {
            allowUnfree = true;
          };
        };
      in
      {
        devShells.default =
          let
            depsBevyRuntime = with pkgs; [
              udev
              alsa-lib-with-plugins
              vulkan-loader
              libxkbcommon
              wayland
              ron-lsp
            ];

            deps = depsBevyRuntime;
          in
          pkgs.mkShell {
            packages = with pkgs; [
              bashInteractive
              readline
            ];
            nativeBuildInputs = [
              pkgs.pkg-config
            ];
            buildInputs = deps;
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath deps;
            shellHook = ''
              export HOSTNAME="nix-shell"
              export PS1="(nix-shell) \w\$ "
              export SHELL=${pkgs.bashInteractive}/bin/bash

              echo "welcome to dev-shell!"
              echo "cargo: $(cargo version)"
              echo "[IDE] rust toolchain path: $(dirname $(which rustc))"
            '';
          };
      }
    );
}
