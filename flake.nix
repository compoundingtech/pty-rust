{
  description = "pty - persistent terminal sessions with detach/attach, in Rust on libghostty";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        inherit (pkgs) lib;

        # Cargo.toml is the single source of truth for the version; a release
        # bump needs no matching edit here.
        version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

        # `crates/pty/build.rs` stamps `<version>+<PTY_BUILD_SHA>` into
        # `pty version`. A flake source carries no `.git`, so the commit comes
        # from the flake's own metadata. A dirty tree reports its base commit,
        # the same way `git rev-parse --short HEAD` does for a local build.
        buildSha = self.shortRev or (lib.removeSuffix "-dirty" (self.dirtyShortRev or "dirty"));

        # The Ghostty commit that libghostty-vt-sys 0.2.1 pins (its build.rs:7).
        # That build script clones it at build time unless GHOSTTY_SOURCE_DIR
        # names a checkout, and lets `zig build` fetch Ghostty's own packages
        # unless GHOSTTY_ZIG_SYSTEM_DIR names a pre-populated package directory.
        # The build sandbox has no network, so both are fixed-output fetches.
        ghosttyRev = "a887df42c56f6de86c0fe6da9c4eeca37931e083";
        ghosttyShortRev = lib.substring 0 7 ghosttyRev;

        ghosttySrc = pkgs.fetchgit {
          name = "ghostty-${ghosttyShortRev}-src";
          url = "https://github.com/ghostty-org/ghostty.git";
          rev = ghosttyRev;
          hash = "sha256-1Zz65SCk3rkJ9+Q0MmyNOTNiDSLBRIHRd3IvFM4iNXw=";
        };

        # Ghostty's zig package cache, in the layout `zig build --system <dir>`
        # reads (one directory per package, named by its content hash). Fetched
        # with Ghostty's own script because `zig build --fetch` skips transitive
        # dependencies (ziglang/zig#20976); the script walks build.zig.zon.txt,
        # the full transitive list Ghostty checks in. The hash is what this
        # recipe produced on 2026-08-29; it is not Ghostty's nix/zigCacheHash.nix,
        # whose flake fetches with a different recipe.
        ghosttyZigDeps = pkgs.stdenvNoCC.mkDerivation {
          name = "ghostty-${ghosttyShortRev}-zig-deps";
          src = ghosttySrc;

          nativeBuildInputs = [
            pkgs.cacert
            pkgs.git
            pkgs.zig_0_15
          ];

          dontConfigure = true;
          dontFixup = true;

          buildPhase = ''
            runHook preBuild
            export ZIG_GLOBAL_CACHE_DIR="$TMPDIR/zig-global-cache"
            ./nix/build-support/fetch-zig-cache.sh
            runHook postBuild
          '';

          installPhase = ''
            runHook preInstall
            mv "$ZIG_GLOBAL_CACHE_DIR/p" "$out"
            runHook postInstall
          '';

          outputHashMode = "recursive";
          outputHash = "sha256-PnM+hZIlLyQwK8vJgd/Bhjt1lNIz06T8FahwliRmMrY=";
        };

        completionShells = [
          "bash"
          "zsh"
          "fish"
        ];

        pty = pkgs.rustPlatform.buildRustPackage {
          pname = "pty";
          inherit version;
          src = self;

          # No git dependencies in the lockfile, so it pins every input on its
          # own; nothing to hand-patch when a dependency bumps.
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [
            pkgs.installShellFiles
            pkgs.zig_0_15
          ];

          # zig's setup hook would otherwise replace cargo's build, check, and
          # install phases; zig is only here for libghostty-vt-sys's build script.
          dontUseZigBuild = true;
          dontUseZigCheck = true;
          dontUseZigInstall = true;

          env = {
            PTY_BUILD_SHA = buildSha;
            GHOSTTY_SOURCE_DIR = "${ghosttySrc}";
            GHOSTTY_ZIG_SYSTEM_DIR = "${ghosttyZigDeps}";
          };

          # Completions are the files vendored from the Node repo, which the
          # binary embeds and prints from `pty completions <shell>`;
          # `checks.completions` proves the two stay identical.
          postInstall = ''
            installShellCompletion --cmd pty \
              --bash completions/pty.bash \
              --zsh completions/pty.zsh \
              --fish completions/pty.fish
          '';

          # The whole workspace runs in the sandbox: openpty works there. The
          # socket tests build their registries under $TMPDIR, and a unix socket
          # path is capped at 108 bytes, so the tests get a short one; the build
          # directory's own name is already a third of that. A few tests read
          # $HOME, and the sandbox HOME is not writable.
          cargoTestFlags = [ "--workspace" ];

          # The testkit's line-editing tests drive readline through `bash`;
          # stdenv's bash is built without it, so the interactive one goes first
          # on the check PATH.
          nativeCheckInputs = [ pkgs.bashInteractive ];

          preCheck = ''
            export TMPDIR=$(mktemp -d /tmp/pty.XXXXXX)
            export HOME=$(mktemp -d)
          '';

          meta = {
            description = "Persistent terminal sessions with detach/attach, hosted by a per-session daemon";
            homepage = "https://github.com/compoundingtech/pty-rust";
            license = lib.licenses.mit;
            mainProgram = "pty";
          };
        };
      in
      {
        packages.pty = pty;
        packages.default = pty;

        # `nix flake check` builds the package (which runs `cargo test
        # --workspace`) and the smoke tests below.
        checks.pty = pty;

        # The installed completion files are the ones the binary prints, byte
        # for byte. Both come from completions/ at the repo root; this proves the
        # embedded copies did not drift from the files.
        checks.completions = pkgs.runCommand "pty-completions-${version}" { } ''
          ${lib.concatMapStringsSep "\n" (shell: ''
            ${pty}/bin/pty completions ${shell} > ${shell}.out
            cmp ${shell}.out ${self}/completions/pty.${shell} \
              || { echo "pty completions ${shell} differs from completions/pty.${shell}" >&2; exit 1; }
          '') completionShells}
          touch $out
        '';

        # The built binary runs, prints the vendored usage text, and carries
        # this flake's commit in its version.
        checks.help = pkgs.runCommand "pty-help-${version}" { } ''
          ${pty}/bin/pty help > help.out
          cmp help.out ${self}/crates/pty/tests/fixtures/help/usage.txt \
            || { echo "pty help differs from the usage fixture" >&2; exit 1; }
          ${pty}/bin/pty version > version.out
          grep -qx '${version}+${buildSha}' version.out \
            || { echo "pty version printed $(cat version.out), expected ${version}+${buildSha}" >&2; exit 1; }
          touch $out
        '';

        devShells.default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.clippy
            pkgs.git
            pkgs.rust-analyzer
            pkgs.rustc
            pkgs.rustfmt
            pkgs.zig_0_15
          ];

          # The same pre-fetched Ghostty as the package, so a `cargo build` in
          # this shell fetches nothing.
          env = {
            GHOSTTY_SOURCE_DIR = "${ghosttySrc}";
            GHOSTTY_ZIG_SYSTEM_DIR = "${ghosttyZigDeps}";
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          };
        };
      }
    );
}
