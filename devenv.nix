{ pkgs, ... }:

{
  dotenv.enable = true;

  packages = with pkgs; [
    lld

    cargo-audit
    cargo-deny
    cargo-expand
    cargo-fuzz
    cargo-release
    cargo-watch
  ];

  languages = {
    rust = {
      enable = true;
    };
  };
}
