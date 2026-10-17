defmodule GpuiVisualTest.MixProject do
  use Mix.Project

  def project do
    [
      app: :gpui_visual_test,
      version: "0.1.0",
      elixir: "~> 1.14",
      start_permanent: Mix.env() == :prod,
      deps: deps(),
      description:
        "Plain-English visual checks and pixel diffs for screenshots in ExUnit, through the cargo visual-test CLI.",
      package: [
        licenses: ["Apache-2.0"],
        links: %{"GitHub" => "https://github.com/Ziinc/gpui-visual-test"}
      ]
    ]
  end

  def application, do: [extra_applications: [:logger]]

  defp deps, do: [{:jason, "~> 1.4"}]
end
