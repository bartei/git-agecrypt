{ pkgs ? import <nixpkgs> { } }:

# grcov used to be vendored here as a `rustPlatform.buildRustPackage` because
# it wasn't in nixpkgs at the time. It is now (v0.9.x), so we just pull it in
# directly — that also drops the deprecated `cargoSha256` field which current
# nixpkgs rejects outright.
pkgs.mkShell {
  nativeBuildInputs = [ pkgs.pkg-config ];
  buildInputs = [
    pkgs.openssl.dev
    pkgs.clang
    pkgs.gdb
    pkgs.lldb
    pkgs.just
    pkgs.grcov
    pkgs.cargo-llvm-cov
    pkgs.cargo-limit
    pkgs.cargo-watch
    pkgs.cargo-audit
  ] ++ pkgs.lib.optional pkgs.stdenv.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
}
