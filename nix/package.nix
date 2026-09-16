{
  lib,
  python3Packages,
}:

python3Packages.buildPythonApplication {
  pname = "xmmrpc";
  version = "0.1.0";
  pyproject = true;

  src = lib.cleanSource ../.;

  build-system = [ python3Packages.hatchling ];

  dependencies = with python3Packages; [
    configargparse
    pyroute2
  ];

  nativeCheckInputs = with python3Packages; [
    pytestCheckHook
  ];

  pythonImportsCheck = [ "xmmrpc" ];

  meta = with lib; {
    description = "Userspace RPC control for Intel XMM7360 modems driven by the in-tree iosm driver";
    homepage = "https://github.com/serephus/xmmrpc";
    license = with licenses; [ gpl2Only bsd3 ];
    platforms = platforms.linux;
    mainProgram = "xmmrpc";
  };
}
