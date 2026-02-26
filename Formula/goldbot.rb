class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.8.3"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.3/goldbot-v0.8.3-macos-aarch64.tar.gz"
      sha256 "9d0d8a7837b3f433cb971f3826602015e19fa9c1a09beec95fb232bf7e367445"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.3/goldbot-v0.8.3-macos-x86_64.tar.gz"
      sha256 "6667f312d1b10b9ba576b55dd1e2facdebd1f9a1d0a9fe428a7e46c7926b160b"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.3/goldbot-v0.8.3-linux-x86_64.tar.gz"
    sha256 "abed4520312382a35e52c0a9cc269ce22440c012c7934406425ea8629c1c27fb"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
