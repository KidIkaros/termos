{ pkgs, ... }:

pkgs.rustPlatform.buildRustPackage {
  pname = "termos";
  version = "0.1.0";

  src = ./.;

  # Update this hash after dependencies change by running the build
  # and copying the expected hash from the error output.
  cargoHash = pkgs.lib.fakeHash;

  # Optional features can be enabled via cargoFeatures.
  # cargoFeatures = [ "network" "tls" ];

  nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux [
    pkgs.pkg-config
  ];

  meta = with pkgs.lib; {
    description = "Terminal multiplexer and window manager (Rust port of TUIOS)";
    homepage = "https://github.com/Gaurav-Gosain/tuios";
    license = licenses.mit;
    maintainers = [ ];
    mainProgram = "termos";
  };
}
