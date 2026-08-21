{ pkgs, ... }:

{
  dotenv.enable = true;

  dagger.enable = true;
  env.DAGGER_X_RELEASE = "v1.0.0-beta.9";

  packages = with pkgs; [
    lld

    cargo-audit
    cargo-deny
    cargo-dist
    cargo-fuzz
    cargo-release
    cargo-watch
    openssl
    pkg-config
  ];

  languages = {
    rust = {
      enable = true;
    };
  };
}
