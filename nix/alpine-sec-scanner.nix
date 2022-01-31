{ buildGoModule, fetchFromGitHub, stdenv, lib }:
buildGoModule {
  name = "alpine-sec-scanner";
  src = fetchFromGitHub {
    owner = "Mic92";
    repo = "alpine-sec-scanner";
    rev = "c97cf8c758ecbd645ea089fb63c5ab371ed74759";
    sha256 = "sha256-s42CJHSsdRF+KBBqxrmSXp8phz1j8HgeZ9dNO6+GZdM=";
  };

  # static linking
  CGO_ENABLED = 0;
  vendorSha256 = "sha256-ldhn9RVV8tUFezTrilq4blfMjMQX0sqEAI+Ul8fVxqY=";

  meta = with lib; {
    description = "Checks installed packages on alpine linux against https://secdb.alpinelinux.org/";
    homepage = "https://github.com/Mic92/alpine-sec-scanner";
    platforms = platforms.unix;
  };
}
