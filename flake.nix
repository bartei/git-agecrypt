{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, ... }:
    {
      # Modern flake schema uses `overlays.default`; the bare `overlay`
      # attribute is deprecated and warns on every `nix flake check`.
      overlays.default = final: _prev: {
        git-agecrypt = final.callPackage ./default.nix { };
      };
    } //
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ self.overlays.default ];
        };
      in
      {
        packages.default = pkgs.git-agecrypt;
        devShells.default = import ./shell.nix { inherit pkgs; };
      });
}
