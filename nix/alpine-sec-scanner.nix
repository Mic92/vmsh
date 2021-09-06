{ buildGoModule, fetchFromGitHub, stdenv, lib }:
buildGoModule {
  name = "alpine-sec-scanner";
  src = fetchFromGitHub {
    owner = "Mic92";
    repo = "alpine-sec-scanner";
    rev = "a1c2f7f318df8d2950740a2444416980cdb32564";
    sha256 = "sha256-AR2qoTxrAvpz2Ye1fHQejq6ZOp8KgRKFuHjVGZKOM/c=";
  };

  # static linking
  CGO_ENABLED = 0;
  vendorSha256 = "sha256-ldhn9RVV8tUFezTrilq4blfMjMQX0sqEAI+Ul8fVxqY=";

  meta = with lib; {
    description = "Checks installed packages on alpine linux against https://secdb.alpinelinux.org/";
    homepage = "https://github.com/Mic92/alpine-sec-scanner0";
    platforms = platforms.unix;
  };
}
