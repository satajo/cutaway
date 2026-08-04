{
  description = "Software architecture viewer and change planner";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # Libraries the GUI (winit/egui/wgpu) loads at runtime via dlopen; they
      # must be on LD_LIBRARY_PATH both in the dev shell and for the installed
      # binary. vulkan-loader serves the wgpu renderer and finds the system's
      # Vulkan drivers itself (on NixOS under /run/opengl-driver).
      guiRuntimeLibs =
        pkgs:
        pkgs.lib.optionals pkgs.stdenv.isLinux (
          with pkgs;
          [
            libGL
            libxkbcommon
            vulkan-loader
            wayland
            libx11
            libxcursor
            libxi
            libxrandr
          ]
        );
    in
    {
      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "cutaway";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.makeWrapper ];
          # The git adapter tests create throwaway repositories with the git CLI.
          nativeCheckInputs = [ pkgs.git ];

          postFixup = pkgs.lib.optionalString pkgs.stdenv.isLinux ''
            wrapProgram $out/bin/cutaway \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath (guiRuntimeLibs pkgs)}
          '';

          meta = {
            description = "Software architecture viewer and change planner";
            homepage = "https://github.com/satajo/cutaway";
            license = pkgs.lib.licenses.gpl3Plus;
            mainProgram = "cutaway";
          };
        };
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            rust-analyzer
            gnumake
            git
          ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (guiRuntimeLibs pkgs);
        };
      });
    };
}
