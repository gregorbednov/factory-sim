{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  nativeBuildInputs = with pkgs; [ cargo rustc rustfmt ];
  LD_LIBRARY_PATH = with pkgs; lib.makeLibraryPath [
    vulkan-loader libglvnd wayland
    libxkbcommon libx11 libxcursor libxi libxrandr
  ] + ":/run/opengl-driver/lib";
}
