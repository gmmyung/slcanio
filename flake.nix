{
  description = "slcanio Rust development environment";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.nixpkgs-darwin.url = "github:NixOS/nixpkgs/nixpkgs-26.05-darwin";

  outputs = { self, nixpkgs, nixpkgs-darwin }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in {
      devShells = forAllSystems (system:
        let
          source = if system == "x86_64-darwin" then nixpkgs-darwin else nixpkgs;
          pkgs = import source { inherit system; };
        in {
          default = pkgs.mkShell {
            packages = with pkgs; [ rustc cargo rustfmt clippy pkg-config ];
          };
        });
    };
}
