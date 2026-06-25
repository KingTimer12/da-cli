#!/bin/sh
# Instalador do DA (Deploy Automático) para Linux e macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/KingTimer12/da-cli/master/install.sh | sh
#
# Variáveis opcionais:
#   DA_VERSION      tag específica (ex: v0.1.0). Default: latest.
#   DA_INSTALL_DIR  diretório de instalação. Default: $HOME/.local/bin.
set -eu

REPO="KingTimer12/da-cli"
BIN="da"

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
  Linux)
    case "$arch" in
      x86_64 | amd64) target="x86_64-unknown-linux-gnu" ;;
      *) echo "arquitetura não suportada no Linux: $arch" >&2; exit 1 ;;
    esac
    ;;
  Darwin)
    case "$arch" in
      arm64 | aarch64) target="aarch64-apple-darwin" ;;
      x86_64) target="x86_64-apple-darwin" ;;
      *) echo "arquitetura não suportada no macOS: $arch" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "SO não suportado: $os (no Windows use install.ps1)" >&2
    exit 1
    ;;
esac

version="${DA_VERSION:-latest}"
if [ "$version" = "latest" ]; then
  url="https://github.com/$REPO/releases/latest/download/$BIN-$target.tar.gz"
else
  url="https://github.com/$REPO/releases/download/$version/$BIN-$target.tar.gz"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "baixando $url"
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$url" -o "$tmp/da.tar.gz"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$tmp/da.tar.gz" "$url"
else
  echo "precisa de curl ou wget instalado" >&2
  exit 1
fi

tar -xzf "$tmp/da.tar.gz" -C "$tmp"

dir="${DA_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$dir"
install -m 0755 "$tmp/$BIN-$target/$BIN" "$dir/$BIN"

echo "instalado em $dir/$BIN"

case ":$PATH:" in
  *":$dir:"*) ;;
  *)
    echo ""
    echo "'$dir' não está no PATH. Adicione ao seu shell rc:"
    echo "  export PATH=\"$dir:\$PATH\""
    ;;
esac

echo "rode: da --help"
