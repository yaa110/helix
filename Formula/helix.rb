class Helix < Formula
  desc "Unofficial AI-assisted Helix fork with VSCode-style tree/search/session features"
  homepage "https://github.com/yaa110/helix"
  url "https://github.com/yaa110/helix.git", branch: "master"
  version "1.0.5"
  license "MPL-2.0"
  head "https://github.com/yaa110/helix.git", branch: "master"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", "--path", "helix-term", "--root", libexec
    (share/"helix/runtime").install Dir["runtime/*"]
    (bin/"hx").write_env_script libexec/"bin/hx", HELIX_RUNTIME: share/"helix/runtime"
  end

  def caveats
    <<~EOS
      This is an unofficial, AI-assisted Helix fork. Issues and pull requests
      are not managed. The runtime files are installed under:
        #{opt_share}/helix/runtime
    EOS
  end

  test do
    system "#{bin}/hx", "--version"
  end
end
