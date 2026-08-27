defmodule Hello.MixProject do
  use Mix.Project

  # `app: :hello` is what the release is named after; the provider parses it and
  # launches /app/bin/hello start.
  def project do
    [
      app: :hello,
      version: "0.1.0",
      elixir: "~> 1.18"
    ]
  end

  def application do
    [
      extra_applications: [:logger],
      mod: {Hello.Application, []}
    ]
  end
end
