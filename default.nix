{
  lib,
  rustPlatform,
}:
let
  cargoToml = fromTOML (builtins.readFile ./Cargo.toml);
  filterSrc =
    src: regexes:
    lib.cleanSourceWith {
      inherit src;
      filter =
        path: type:
        let
          relPath = lib.removePrefix (toString src + "/") (toString path);
        in
        lib.all (re: builtins.match re relPath == null) regexes;
    };
in
rustPlatform.buildRustPackage {
  pname = cargoToml.package.name;
  version = cargoToml.package.version;

  src = filterSrc ./. [
    ".*\\.nix$"
    "^flake\\.lock$"
    "^target($|/.*)"
    "^\\.git($|/.*)"
    "^\\.direnv($|/.*)"
    "^\\.envrc$"
    "^\\.github($|/.*)"
  ];

  cargoLock.lockFile = ./Cargo.lock;

  env = {
    RUST_BACKTRACE = 1;
    CARGO_INCREMENTAL = "0";
  };

  meta = {
    description = cargoToml.package.description;
    homepage = cargoToml.package.homepage;
    license = lib.licenses.gpl3Only;
    mainProgram = cargoToml.package.name;
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
}
