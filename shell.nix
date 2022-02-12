with import <nixpkgs> {};

stdenv.mkDerivation {
  name = "rust-env";
  nativeBuildInputs = [
    rustc cargo clippy rustfmt rust-analyzer
    pkgconfig
  ];
  buildInputs = [
    gpgme
    libudev
    openssl
  ];

  # Set Environment Variables
  RUST_BACKTRACE = 1;
}
