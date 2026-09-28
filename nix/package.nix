{
  lib,
  rustPlatform,
}:

rustPlatform.buildRustPackage {
  pname = "xmmrpc";
  version = "0.1.0";

  src = lib.cleanSource ../.;
  cargoLock.lockFile = ../Cargo.lock;

  meta = {
    description = "Userspace RPC control for Intel XMM7360 (Fibocom L850-GL) modems driven by the in-tree iosm driver";
    homepage = "https://github.com/serephus/xmmrpc";
    license = with lib.licenses; [ gpl2Only bsd3 ];
    platforms = lib.platforms.linux;
    mainProgram = "xmmrpc";
  };
}
