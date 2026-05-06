class Sifs < Formula
  desc "SIFS Is Fast Search: instant local code search for agents"
  homepage "https://github.com/tristanmanchester/sifs"
  url "https://github.com/tristanmanchester/sifs/archive/refs/tags/v0.3.2.tar.gz"
  sha256 "de1fa8a92c82a159b41363c7fa1e5c90fe9ca0cd0700b6c9c24e389b0150699b"
  license "MIT"
  head "https://github.com/tristanmanchester/sifs.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", "--path", ".", "--root", prefix
  end

  test do
    repo = testpath/"tiny-repo"
    repo.mkpath
    (repo/"auth.py").write <<~PY
      def authenticate_token(token):
          return token == "secret"
    PY

    system "git", "-C", repo, "init", "--quiet"

    output = shell_output("#{bin}/sifs search authenticate_token --source #{repo} --mode bm25 --offline --no-cache")
    assert_match "auth.py", output
    assert_match "authenticate_token", output
  end
end
