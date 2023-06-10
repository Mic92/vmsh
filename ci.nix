let
  checks = (builtins.getFlake (toString ./.)).checks.x86_64-linux;
in
builtins.removeAttrs checks [ "nixos-image" ]
