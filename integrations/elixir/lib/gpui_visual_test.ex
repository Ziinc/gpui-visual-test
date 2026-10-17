defmodule GpuiVisualTest do
  @moduledoc """
  Visual checks for screenshots, judged by gpui-visual-test.

  Hands a PNG and plain-English expectations to `cargo visual-test check
  --json`, so verdict reuse, agent review and pixel diffs against approved
  baselines work the same as in Rust tests. Use `GpuiVisualTest.Assertions`
  in ExUnit.

  ## Options

    * `:expect` - expectations that must hold
    * `:should` - expectations reported but never failing
    * `:expect_in` - expectations about one region, `[toolbar: "Save is grey"]`
      or `%{"toolbar" => ["...", "..."]}`
    * `:regions` - named areas in logical pixels, `[toolbar: {x, y, w, h}]`
    * `:scale` - physical pixels per logical pixel of the image (default 1)
    * `:diff` - pixel-diff against the approved baseline, allowing this share
      (0 to 1) of pixels to change
    * `:baseline_dir`, `:update_baseline` - for the diff
    * `:judge` - judge spec, such as `"openai:<model>"`; default
      `$GPUI_VISUAL_JUDGE` or `agent`
    * `:rejudge` - ignore stored verdicts
    * `:dir` - output directory; default `$GPUI_VISUAL_DIR` or `target/visual-test`
    * `:cli` - the CLI to run; default `$GPUI_VISUAL_TEST_CLI` or `cargo-visual-test`
  """

  @type result :: map()
  @type outcome :: {:ok, result} | {:fail, result} | {:pending, result} | {:error, String.t()}

  @doc """
  Checks `image` (PNG bytes, or `{:file, path}`) as shot `name`.

  Returns `{:ok, result}`, `{:fail, result}`, `{:pending, result}` (an
  expectation or baseline needs a decision) or `{:error, message}`. `result`
  is the CLI's JSON manifest with string keys.
  """
  @spec check(binary | {:file, Path.t()}, String.t(), keyword) :: outcome
  def check(image, name, opts \\ []) do
    with {:ok, cli} <- find_cli(opts),
         {:ok, args} <- args(name, opts) do
      with_file(image, fn path -> run(cli, args.(path)) end)
    end
  end

  @doc false
  def args(name, opts) do
    if name in [nil, ""] do
      {:error, "a visual check needs a shot name, such as \"counter/initial\""}
    else
      flags =
        Enum.flat_map(List.wrap(opts[:expect]), &["--expect", &1]) ++
          Enum.flat_map(List.wrap(opts[:should]), &["--should", &1]) ++
          Enum.flat_map(opts[:expect_in] || [], fn {region, texts} ->
            Enum.flat_map(List.wrap(texts), &["--expect-in", "#{region}=#{&1}"])
          end) ++
          Enum.flat_map(opts[:regions] || [], fn {id, {x, y, w, h}} ->
            ["--region", "#{id}=#{x},#{y},#{w},#{h}"]
          end) ++
          diff_flags(opts) ++
          if(opts[:judge], do: ["--judge", opts[:judge]], else: []) ++
          if(opts[:rejudge], do: ["--rejudge"], else: [])

      dir = if opts[:dir], do: ["--dir", to_string(opts[:dir])], else: []
      scale = to_string(opts[:scale] || 1)

      {:ok,
       fn path ->
         dir ++ ["check", path, "--name", name, "--scale", scale, "--json"] ++ flags
       end}
    end
  end

  # Baseline options mean nothing without a diff, so shared defaults can
  # carry them.
  defp diff_flags(opts) do
    case opts[:diff] do
      nil ->
        []

      max ->
        ["--diff", to_string(max)] ++
          if(opts[:baseline_dir], do: ["--baseline-dir", to_string(opts[:baseline_dir])], else: []) ++
          if(opts[:update_baseline], do: ["--update-baseline"], else: [])
    end
  end

  defp find_cli(opts) do
    cli = opts[:cli] || System.get_env("GPUI_VISUAL_TEST_CLI") || "cargo-visual-test"

    cond do
      File.exists?(cli) ->
        {:ok, cli}

      path = System.find_executable(cli) ->
        {:ok, path}

      true ->
        {:error,
         "could not run #{cli}. Install it with `cargo install --git " <>
           "https://github.com/Ziinc/gpui-visual-test cargo-visual-test` or set GPUI_VISUAL_TEST_CLI."}
    end
  end

  defp with_file({:file, path}, fun), do: fun.(Path.expand(path))

  defp with_file(png, fun) when is_binary(png) do
    path =
      Path.join(System.tmp_dir!(), "gpui-visual-test-#{System.unique_integer([:positive])}.png")

    File.write!(path, png)

    try do
      fun.(path)
    after
      File.rm(path)
    end
  end

  defp run(cli, args) do
    # With --json the CLI writes only JSON to stdout and only errors to
    # stderr, so one merged stream keeps the error text.
    {out, code} = System.cmd(cli, args, stderr_to_stdout: true)

    case {code, Jason.decode(out)} do
      {0, {:ok, result}} -> {:ok, result}
      {1, {:ok, result}} -> {:fail, result}
      {2, {:ok, result}} -> {:pending, result}
      {code, _} -> {:error, "cargo visual-test failed (exit #{code}): #{String.trim(out)}"}
    end
  end

  @doc "A readable report of a failed or pending result."
  @spec describe(result) :: String.t()
  def describe(result) do
    diff =
      case result["diff"] do
        %{"status" => "pass"} ->
          []

        nil ->
          []

        d ->
          ["  [diff] #{d["status"]}: #{d["reason"]}"] ++
            if(d["image"], do: ["         diff image: #{d["image"]}"], else: [])
      end

    checks =
      for c <- result["checks"], c["status"] != "pass" do
        reason = if c["verdict"], do: " (#{c["verdict"]["reason"]})", else: ""
        "  [#{c["index"]}] #{c["status"]}: #{c["text"]}#{reason}"
      end

    hint =
      if result["status"] == "unsure",
        do: [
          "Look at the image, then record decisions with `cargo visual-test verdict` " <>
            "(or approve a baseline with `cargo visual-test approve`), " <>
            "or set GPUI_VISUAL_JUDGE to a model judge."
        ],
        else: []

    Enum.join(
      ["visual check #{inspect(result["name"])} #{result["status"]}: #{result["image"]}"] ++
        diff ++ checks ++ hint,
      "\n"
    )
  end
end
