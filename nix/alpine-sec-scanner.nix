{ buildGoModule, fetchFromGitHub, stdenv }:
buildGoModule {
  name = "alpine-sec-scanner";
  src = fetchFromGitHub {
    owner = "Mic92";
    repo = "alpine-sec-scanner";
    rev = "a1c2f7f318df8d2950740a2444416980cdb32564";
    sha256 = "0000000000000000000000000000000000000000000000000000";
  };

  vendorSha256 = "0c3ly0s438sr9iql2ps4biaswphp7dfxshddyw5fcm0ajqzvhrmw";

  meta = with stdenv.lib; {
    description = "Checks installed packages on alpine linux against https://secdb.alpinelinux.org/";
    homepage = "https://github.com/Mic92/alpine-sec-scanner0";
    platforms = platforms.unix;
  };
}
