# Sourced by the lab scripts. Everything the servers need lives under lab/servers/ (git-ignored).
LAB_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVERS="$LAB_ROOT/servers"
DL="$SERVERS/dl"
JDKS="${JDKS:-$HOME/.local/jdks}"   # Temurin JDKs/JREs; a missing one is downloaded here
LAB_HEAP="${LAB_HEAP:-896M}"
# Test accounts (offline mode): opped on both servers; the first one gets auto-fly.
LAB_OPS="${LAB_OPS:-GhastBot GhastBot2 GhastBot3 GhastBot4 GhastBot5 GhastBot6 GhastBot7 GhastBot8 GhastBot9 GhastBot10 GhastBot11 GhastBot13 Baseline Baseline2}"
LAB_BOT="${LAB_BOT:-GhastBot}"
# Accounts the lab /fly plugin gives allowFlight on join (read at server start).
LAB_FLY="${LAB_FLY:-GhastBot GhastBot5}"

GRIM_JAR=grimac-bukkit-2.3.73.jar
GRIM_URL=https://cdn.modrinth.com/data/LJNGWSvH/versions/1FIGlM6Q/grimac-bukkit-2.3.73.jar
GRIM_SHA=a095331a8ba47286b27bdb9b51f0d35ae4a227632a2c50b25cedba85439f1810

# lab_select <1.8|1.12|1.16|26.1|26.2>: sets VER, GAME (the client's version name), DIR, PORT, PAPER_JAR,
# PAPER_URL, PAPER_SHA, JAVA, GRIM_*
lab_select() {
  case "${1:-}" in
    1.8|1.8.8|1.8.9|18)
      VER=1.8; PORT=25618; JAVA_MAJOR=21; JAVA_FLAGS="-DIReallyKnowWhatIAmDoingISwear=true"
      PAPER_JAR=paper-1.8.8-445.jar
      PAPER_SHA=7ff6d2cec671ef0d95b3723b5c92890118fb882d73b7f8fa0a2cd31d97c55f86 ;;
    1.12|1.12.2|112)
      # Grim 2.3.73 needs Java 17; Paper 1.12.2 (last build 1620) runs on it when told to ignore it
      VER=1.12; PORT=25612; JAVA_MAJOR=17; JAVA_FLAGS="-DPaper.IgnoreJavaVersion=true"
      PAPER_JAR=paper-1.12.2-1620.jar
      PAPER_SHA=3a2041807f492dcdc34ebb324a287414946e3e05ec3df6fd03f5b5f7d9afc210 ;;
    1.16|1.16.5|116)
      # Grim 2.3.73 needs Java 17; Paper 1.16.5 runs on it but refuses it without -DPaper.IgnoreJavaVersion
      VER=1.16; PORT=25616; JAVA_MAJOR=17; JAVA_FLAGS="-DPaper.IgnoreJavaVersion=true"
      PAPER_JAR=paper-1.16.5-794.jar
      PAPER_SHA=e67da4851d08cde378ab2b89be58849238c303351ed2482181a99c2c2b489276 ;;
    26.1|26.1.1|26.1.2|261)
      # Protocol 775 (26.1 - 26.1.2): Paper 26.1.2 (final build 74) needs Java 25; Grim 2.3.74 is the
      # first release with native 26.x support (no ViaVersion).
      VER=26.1; PORT=25626; JAVA_MAJOR=25; JAVA_FLAGS=""
      PAPER_JAR=paper-26.1.2-74.jar
      PAPER_SHA=1d70b1dab9cf4a6de615209a536f3a45a2186240253c428213ce2188ab95e5f7
      GRIM_JAR=grimac-bukkit-2.3.74.jar
      GRIM_URL=https://cdn.modrinth.com/data/LJNGWSvH/versions/baDrVvfo/grimac-bukkit-2.3.74.jar
      GRIM_SHA=3ac71c0b6ef8ab586c1dd3ab4c0b3254394ef39d048e4985efb2898b9fef628c ;;
    26.2|262)
      # Protocol 776 (26.2): Paper 26.2 build 135 (stable), Java 25; Grim 2.3.74 lists 26.2.
      VER=26.2; PORT=25634; JAVA_MAJOR=25; JAVA_FLAGS=""
      PAPER_JAR=paper-26.2-135.jar
      PAPER_SHA=ef3dbc68b013deafdbbee3bebf1ba351671ebcda94a8f136809b7532d5ba2fe1
      GRIM_JAR=grimac-bukkit-2.3.74.jar
      GRIM_URL=https://cdn.modrinth.com/data/LJNGWSvH/versions/baDrVvfo/grimac-bukkit-2.3.74.jar
      GRIM_SHA=3ac71c0b6ef8ab586c1dd3ab4c0b3254394ef39d048e4985efb2898b9fef628c ;;
    *) echo "usage: $(basename "$0") <1.8|1.12|1.16|26.1|26.2> ..." >&2; exit 2 ;;
  esac
  PAPER_URL="https://fill-data.papermc.io/v1/objects/$PAPER_SHA/$PAPER_JAR"
  DIR="$SERVERS/$VER"
  case "$VER" in 1.8) GAME=1.8.9 ;; 1.12) GAME=1.12.2 ;; 1.16) GAME=1.16.5 ;; *) GAME=$VER ;; esac
}

is_modern() { # the selected lab runs a 26.x server
  case "$VER" in 26.*) return 0 ;; *) return 1 ;; esac
}

java_home() { # java_home <major>: prints a Temurin home for that major, downloading it on first use
  local m=$1 d
  d=$(ls -d "$JDKS"/jdk-"$m".* "$JDKS"/jdk-"$m"+* "$HOME/.local/jdk$m" 2>/dev/null | sort -V | tail -1)
  if [ -z "$d" ] || [ ! -x "$d/bin/java" ]; then
    mkdir -p "$JDKS/dl"; local t="$JDKS/dl/temurin-$m.tar.gz"
    curl -fsSL -o "$t" "https://api.adoptium.net/v3/binary/latest/$m/ga/linux/x64/jdk/hotspot/normal/eclipse" >&2
    d="$JDKS/$(tar -tzf "$t" | head -1 | cut -d/ -f1)"; tar -xzf "$t" -C "$JDKS"; rm -f "$t"
  fi
  echo "$d"
}

lab_pid() { # prints the server's pid if it is running
  local p; p=$(cat "$DIR/server.pid" 2>/dev/null) || return 1
  [ -n "$p" ] && kill -0 "$p" 2>/dev/null && grep -qa "$PAPER_JAR" "/proc/$p/cmdline" 2>/dev/null && echo "$p"
}

fetch() { # fetch <url> <dest> <sha256>
  [ -f "$2" ] && echo "$3  $2" | sha256sum -c --quiet 2>/dev/null && return 0
  mkdir -p "$(dirname "$2")"
  curl -fsSL -o "$2.part" "$1"
  echo "$3  $2.part" | sha256sum -c --quiet
  mv "$2.part" "$2"
}

offline_uuid() { # Java UUID.nameUUIDFromBytes("OfflinePlayer:<name>")
  python3 -c 'import hashlib,sys,uuid; b=bytearray(hashlib.md5(("OfflinePlayer:"+sys.argv[1]).encode()).digest()); b[6]=b[6]&0x0f|0x30; b[8]=b[8]&0x3f|0x80; print(uuid.UUID(bytes=bytes(b)))' "$1"
}
