{
  description = "fold — one outline of notes and tasks in plain Markdown, in the terminal";

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
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      mkFold =
        pkgs:
        pkgs.rustPlatform.buildRustPackage {
          pname = "fold";
          version = "0.1.0";
          # only what the build reads, so editing SPEC.md does not rebuild
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./crates
            ];
          };
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "-p" "fold-cli" ];
          # the test suite runs in `nix flake check` (checks.default)
          doCheck = false;
          meta = {
            description = "One outline of notes and tasks in plain Markdown, in the terminal";
            mainProgram = "fold";
            platforms = systems;
          };
        };
    in
    {
      packages = forAll (pkgs: rec {
        fold = mkFold pkgs;
        default = fold;
      });

      # `nix flake check`: the whole workspace's tests
      checks = forAll (pkgs: {
        default = (mkFold pkgs).overrideAttrs (_: {
          pname = "fold-tests";
          doCheck = true;
          cargoTestFlags = [ "--workspace" ];
          # the trash lives under $XDG_STATE_HOME; the sandbox has no home
          preCheck = ''
            export HOME=$(mktemp -d)
            export XDG_STATE_HOME=$HOME/.local/state
          '';
        });
      });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ (mkFold pkgs) ];
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            rust-analyzer
          ];
        };
      });
    };
}
