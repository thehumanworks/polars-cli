{
  lib,
  rustPlatform,
  pkg-config,
}:

let
  cargoToml = builtins.fromTOML (builtins.readFile ../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = "polars-cli";
  version = cargoToml.package.version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [ pkg-config ];

  # Tests run in the dev shell (`cargo test`). Building them here pulls a
  # second copy of the Polars dependency tree.
  doCheck = false;

  meta = {
    description = "Bash-native Polars pipelines";
    homepage = "https://github.com/thehumanworks/polars-cli";
    mainProgram = "pl";
    platforms = [
      "x86_64-linux"
      "aarch64-linux"
      "x86_64-darwin"
      "aarch64-darwin"
    ];
  };
}
