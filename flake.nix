{
  description = "Userspace RPC control for Intel XMM7360 (Fibocom L850-GL) modems driven by the in-tree iosm driver";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    let
      xmmrpcFor = pkgs: pkgs.callPackage ./nix/package.nix { };
    in
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        xmmrpc = xmmrpcFor pkgs;
        python = pkgs.python3;
        pythonEnv = python.withPackages (ps: with ps; [
          configargparse
          pyroute2
          pytest
        ]);
      in
      {
        packages = {
          inherit xmmrpc;
          default = xmmrpc;
        };

        checks = {
          inherit xmmrpc;
        };

        devShells.default = pkgs.mkShell {
          name = "xmmrpc";
          buildInputs = [
            python
            pythonEnv
            pkgs.basedpyright
            pkgs.black
            pkgs.ruff
            pkgs.uv
          ];

          # Force uv to use the Python interpreter provided by Nix.
          UV_PYTHON_DOWNLOADS = "never";
          UV_PYTHON = nixpkgs.lib.getExe python;
        };
      }
    )
    // {
      overlays.default = final: prev: {
        xmmrpc = xmmrpcFor final;
      };

      nixosModules.default = import ./nix/module.nix;
    };
}
