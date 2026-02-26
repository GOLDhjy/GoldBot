class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.8.5"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.5/goldbot-v0.8.5-macos-aarch64.tar.gz"
      sha256 "fcf6e41dd0380f437584ee987e881841ca5d06bef230b4dedf66c9a66acc8e3e"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.5/goldbot-v0.8.5-macos-x86_64.tar.gz"
      sha256 "7995a19a6ab15d3c82548c5c44beacbd8d5687a8b55627df09654b9da0d73bfa"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.5/goldbot-v0.8.5-linux-x86_64.tar.gz"
    sha256 "9a330255286837d9295387755fc4e3995f90507ced8788cfb25095dfe0e79eb7"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
