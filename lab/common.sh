# Sourced by the lab scripts. Everything the servers need lives under lab/servers/ (git-ignored).
LAB_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVERS="$LAB_ROOT/servers"
DL="$SERVERS/dl"
JDKS="${JDKS:-$HOME/.local/jdks}"   # Temurin JDKs/JREs; a missing one is downloaded here
LAB_HEAP="${LAB_HEAP:-896M}"
# Test accounts (offline mode): opped on both servers; the first one gets auto-fly.
LAB_OPS="${LAB_OPS:-GhastBot GhastBot2 GhastBot3 GhastBot4 GhastBot5 GhastBot6 GhastBot7 GhastBot8 Baseline Baseline2}"
LAB_BOT="${LAB_BOT:-GhastBot}"
# Accounts the lab /fly plugin gives allowFlight on join (read at server start).
LAB_FLY="${LAB_FLY:-GhastBot GhastBot5}"

GRIM_JAR=grimac-bukkit-2.3.73.jar
GRIM_URL=https://cdn.modrinth.com/data/LJNGWSvH/versions/1FIGlM6Q/grimac-bukkit-2.3.73.jar
GRIM_SHA=a095331a8ba47286b27bdb9b51f0d35ae4a227632a2c50b25cedba85439f1810

# lab_select <1.8|1.16>: sets VER, DIR, PORT, PAPER_JAR, PAPER_URL, PAPER_SHA, JAVA
lab_select() {
  case "${1:-}" in
    1.8|1.8.8|1.8.9|18)
      VER=1.8; PORT=25618; JAVA_MAJOR=21; JAVA_FLAGS="-DIReallyKnowWhatIAmDoingISwear=true"
      PAPER_JAR=paper-1.8.8-445.jar
      PAPER_SHA=7ff6d2cec671ef0d95b3723b5c92890118fb882d73b7f8fa0a2cd31d97c55f86 ;;
    1.16|1.16.5|116)
      # Grim 2.3.73 needs Java 17; Paper 1.16.5 runs on it but refuses it without -DPaper.IgnoreJavaVersion
      VER=1.16; PORT=25616; JAVA_MAJOR=17; JAVA_FLAGS="-DPaper.IgnoreJavaVersion=true"
      PAPER_JAR=paper-1.16.5-794.jar
      PAPER_SHA=e67da4851d08cde378ab2b89be58849238c303351ed2482181a99c2c2b489276 ;;
    *) echo "usage: $(basename "$0") <1.8|1.16> ..." >&2; exit 2 ;;
  esac
  PAPER_URL="https://fill-data.papermc.io/v1/objects/$PAPER_SHA/$PAPER_JAR"
  DIR="$SERVERS/$VER"
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
