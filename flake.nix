{
  description = "apple-mail-mcp - MCP server exposing Apple Mail read/search/send tools";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        # rustc/cargo come from nixpkgs-unstable (unpinned) — MSRV is 1.88
        # (Cargo.toml [workspace.package].rust-version); if this toolchain
        # drifts below that floor, `nix develop` no longer matches CI.
        nativeBuildDeps = [
          pkgs.rustc
          pkgs.cargo
          pkgs.rustfmt
          pkgs.clippy
        ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.darwin.cctools
        ];

      in
      {
        # Development shell: full Rust toolchain for building apple-mail-mcp.
        # On macOS, Foundation/AddressBook/CoreServices frameworks are
        # automatically available via xcrun without listing them explicitly.
        # Usage: nix develop
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = nativeBuildDeps;

          shellHook = ''
            echo "apple-mail-mcp development shell"
            echo "  cargo build --workspace              — debug build"
            echo "  cargo build --profile dist -p amx-cli — release binary → target/dist/amxcli"
            echo "  cargo run --bin amxcli -- <command>   — run from source"
          '';
        };
      }
    );
}
