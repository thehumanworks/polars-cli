{
  description = "pl — Bash-native Polars pipelines";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # Nixpkgs 26.11 dropped x86_64-darwin. The 26.05 Darwin branch still
    # provides it, including security fixes through the end of 2026.
    nixpkgs-x86_64-darwin.url = "github:NixOS/nixpkgs/nixpkgs-26.05-darwin";
  };

  outputs =
    {
      self,
      nixpkgs,
      nixpkgs-x86_64-darwin,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      pkgsFor =
        system:
        if system == "x86_64-darwin" then
          nixpkgs-x86_64-darwin.legacyPackages.${system}
        else
          nixpkgs.legacyPackages.${system};
      forAllSystems = fn: nixpkgs.lib.genAttrs systems (system: fn (pkgsFor system));
      packageFor = pkgs: pkgs.callPackage ./nix/package.nix { };
    in
    {
      overlays.default = final: _prev: {
        polars-cli = packageFor final;
      };

      packages = forAllSystems (
        pkgs:
        let
          polars-cli = packageFor pkgs;
        in
        {
          inherit polars-cli;
          default = polars-cli;
        }
      );

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/pl";
        };
        polars = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/polars";
        };
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            rustc
            cargo
            rustfmt
            clippy
            rust-analyzer
            pkg-config
            stdenv.cc
          ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });

      nixosModules.default =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        let
          cfg = config.programs.pl;
        in
        {
          options.programs.pl = {
            enable = lib.mkEnableOption "pl, the Bash-native Polars CLI";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              defaultText = lib.literalExpression "polars-cli.packages.\${pkgs.stdenv.hostPlatform.system}.default";
              description = "Package that provides the pl and polars binaries.";
            };
          };

          config = lib.mkIf cfg.enable {
            environment.systemPackages = [ cfg.package ];
          };
        };

      homeManagerModules.default =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        let
          cfg = config.programs.pl;
        in
        {
          options.programs.pl = {
            enable = lib.mkEnableOption "pl, the Bash-native Polars CLI";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              defaultText = lib.literalExpression "polars-cli.packages.\${pkgs.stdenv.hostPlatform.system}.default";
              description = "Package that provides the pl and polars binaries.";
            };
          };

          config = lib.mkIf cfg.enable {
            home.packages = [ cfg.package ];
          };
        };
    };
}
