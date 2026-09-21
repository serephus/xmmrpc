{
  description = "Userspace RPC control for Intel XMM7360 (Fibocom L850-GL) modems driven by the in-tree iosm driver";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    nixit.url = "github:serephus/nixit";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      nixit,
      rust-overlay,
      ...
    }:
    {
      githubRepositories.xmmrpc = nixit.lib.githubRepository {
        owner = "serephus";
        name = "xmmrpc";

        description = "Userspace RPC control for Intel XMM7360 (Fibocom L850-GL) modems driven by the in-tree iosm driver";
        homepage = "https://github.com/serephus/xmmrpc";
        topics = [
          "rust"
          "xmm7360"
          "iosm"
          "wwan"
          "lte"
          "fibocom"
          "l850"
        ];
        visibility = "public";

        features = {
          wiki.enable = false;
          issues.enable = true;
          projects.enable = false;
          discussions.enable = false;
        };

        is_template = false;
        is_archived = false;

        pull = {
          merge.enable = true;
          squash.enable = false;
          rebase.enable = false;
          auto_merge = true;
          delete_branch_on_merge = true;
          update_branch = true;
        };

        actions = {
          enable = true;
          policy = "all";
          default_token_permissions = "read";
          allow_pr_approval = false;
        };

        rulesets = {
          default = {
            enforcement = "active";
            conditions.ref_name.include = [ "~DEFAULT_BRANCH" ];
            rules = [
              { type = "deletion"; }
              { type = "non_fast_forward"; }
            ];
          };
          pr = {
            enforcement = "active";
            conditions.ref_name.include = [ "~DEFAULT_BRANCH" ];
            rules = [
              {
                type = "required_status_checks";
                parameters = {
                  strict_required_status_checks_policy = true;
                  required_status_checks = [
                    { "context" = "ubuntu-latest-x86_64-unknown-linux-gnu-nightly"; }
                    { "context" = "ubuntu-latest-x86_64-unknown-linux-gnu-stable"; }
                    { "context" = "Nix Build"; }
                  ];
                };
              }
            ];
          };
        };
      };

      overlays.default = final: _prev: {
        xmmrpc = final.callPackage ./nix/package.nix { };
      };

      nixosModules.default = import ./nix/module.nix;
    }
    // flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rustfmt"
            "clippy"
            "rust-analyzer"
          ];
        };

        xmmrpc = pkgs.callPackage ./nix/package.nix { };
      in
      {
        packages.default = xmmrpc;

        checks.default = xmmrpc;

        devShells.default = pkgs.mkShell {
          name = "xmmrpc";
          buildInputs = [
            rust
            pkgs.cargo-nextest
            pkgs.taplo
          ];
        };
      }
    );
}
