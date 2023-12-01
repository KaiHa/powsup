with import <nixpkgs> {};

stdenv.mkDerivation {
  name = "rust-env";
  nativeBuildInputs = [
    rustc cargo clippy rustfmt rust-analyzer
    pkg-config socat
  ];
  buildInputs = [
    gpgme
    udev
    openssl
  ];

  # Set Environment Variables
  RUST_BACKTRACE = 1;
}
