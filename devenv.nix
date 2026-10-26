# Toolchain, Vulkan and system libraries come from the shared Bevy module
# (chousx/devenv/bevy, see devenv.yaml).
{ pkgs, ... }:
{
  # devenv's rust module sets RUSTFLAGS itself, which drops the shared
  # module's `-fuse-ld=mold`; without this tir links with GNU ld.
  languages.rust.mold.enable = true;

  # `dx serve --hot-patch` (Dioxus' subsecond). Its version must match the
  # `dioxus-devtools` Bevy's `hotpatching` resolves to in Cargo.lock.
  packages = [ pkgs.dioxus-cli ];

  # The grove under `dx serve --hot-patch`: save a source file and it patches
  # in while running (see README). Assets come from the repository root,
  # since dx runs the binary from its own directory.
  scripts.grove-live.exec = ''
    cd "$DEVENV_ROOT"
    # dx needs to see the binary's link; when cargo finds it up to date it
    # fails with "Missing linker args for fat link".
    touch examples/grove.rs
    BEVY_ASSET_ROOT="$DEVENV_ROOT" exec dx serve --hot-patch --package ornithochory-live --bin grove "$@"
  '';
}
