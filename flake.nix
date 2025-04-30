{
  description = "gh-notifier devel and build";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";

  # shell.nix compatibility
  inputs.flake-compat.url = "https://flakehub.com/f/edolstra/flake-compat/1.tar.gz";

  outputs = { self, nixpkgs, ... }:
    let
      # System types to support.
      targetSystems = [ "x86_64-linux" "aarch64-linux" ];

      # Helper function to generate an attrset '{ x86_64-linux = f "x86_64-linux"; ... }'.
      forAllSystems = nixpkgs.lib.genAttrs targetSystems;

      inherit (nixpkgs) lib;
      sharedOptionModule = { lib, pkgs, ... }: {
        options.services.gh-notifier = {
          enable = lib.mkEnableOption "GitHub Notifications notifier for Linux";
          package = lib.mkOption {
            type = lib.types.package;
            default = self.packages.${pkgs.system}.default;
            defaultText = lib.literalMD "`gh-notifier` from the flake defining this module";
            description = ''
              Package to use.
            '';
          };
          systemdTarget = lib.mkOption {
            type = lib.types.str;
            default = "graphical-session.target";
            example = "sway-session.target";
            description = ''
              Systemd target to bind to.
            '';
          };
          environmentFile = lib.mkOption {
            type = lib.types.path;
            description = ''
              The full path to a file which contains environment variables as defined in {manpage}`systemd.exec(5)`.

              The GitHub token ({env}`GITHUB_TOKEN`) should be specified in the
              file this option points to. The token can either be a classic
              personal access token (PAT) or an OAuth app's access token.

              Create a matching classic PAT
              [here](https://github.com/settings/tokens/new?description=gh-notifier&scopes=read%3Auser%2Cnotifications%2Crepo).

              Example file contents:
              ```
              GITHUB_TOKEN=ghp_...
              ```
            '';
            example = "/run/secrets/github-notifier.env";
          };
        };
      };
    in {
      packages = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.rustPlatform.buildRustPackage rec {
            pname = "gh-notifier";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

            src = ./.;

            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = with pkgs; [
              pkg-config
              makeBinaryWrapper
            ];

            buildInputs = with pkgs; [
              openssl
            ];

            postInstall = ''
              wrapProgram $out/bin/gh-notifier \
                --suffix PATH : ${nixpkgs.lib.makeBinPath [ pkgs.xdg-utils ]}
            '';

            meta = with nixpkgs.lib; {
              description = "GitHub Notifications notifier for Linux";
              homepage = "https://github.com/axelkar/gh-notifier";
              license = licenses.asl20;
            };
          };
        }
      );
      devShells = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            strictDeps = true;
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            nativeBuildInputs = with pkgs; [
              cargo
              rustc
              pkg-config

              rustfmt
              clippy
              rust-analyzer
            ];

            inherit (self.packages.${system}.default) buildInputs;
          };
        }
      );
      nixosModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.services.gh-notifier;
        in
        {
          imports = [ sharedOptionModule ];

          config = lib.mkIf cfg.enable {
            systemd.user.services.gh-notifier = {
              description = "GitHub Notifications notifier for Linux";
              partOf = [ cfg.systemdTarget ];
              after = [ cfg.systemdTarget ]; # Make sure a notification daemon is running

              serviceConfig = {
                Restart = "on-failure";
                EnvironmentFile = [ cfg.environmentFile ];
                ExecStart = "${cfg.package}/bin/gh-notifier";
              };

              wantedBy = [ cfg.systemdTarget ];
            };
          };
        };
      homeModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.services.gh-notifier;
        in
        {
          imports = [ sharedOptionModule ];

          config = lib.mkIf cfg.enable {
            systemd.user.services.gh-notifier = {
              Unit = {
                Description = "GitHub Notifications notifier for Linux";
                PartOf = [ cfg.systemdTarget ];
                After = [ cfg.systemdTarget ]; # Make sure a notification daemon is running
              };

              Service = {
                Restart = "on-failure";
                EnvironmentFile = [ cfg.environmentFile ];
                ExecStart = "${cfg.package}/bin/gh-notifier";
              };

              Install.WantedBy = [ cfg.systemdTarget ];
            };
          };
        };
    };
}
