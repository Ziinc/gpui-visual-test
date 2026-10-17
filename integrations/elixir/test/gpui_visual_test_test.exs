# Runs against the real CLI. Build it first (`cargo build -p cargo-visual-test`)
# or set GPUI_VISUAL_TEST_CLI.
defmodule GpuiVisualTestTest do
  use ExUnit.Case, async: true
  import GpuiVisualTest.Assertions

  @cli System.get_env("GPUI_VISUAL_TEST_CLI") ||
         Path.expand("../../../target/debug/cargo-visual-test", __DIR__)

  defp png(w, h, square \\ nil) do
    rows =
      for y <- 0..(h - 1), into: <<>> do
        row =
          for x <- 0..(w - 1), into: <<>> do
            dark =
              case square do
                {sx, sy, s} -> x >= sx and x < sx + s and y >= sy and y < sy + s
                nil -> false
              end

            if dark, do: <<20, 20, 20, 255>>, else: <<255, 255, 255, 255>>
          end

        <<0>> <> row
      end

    chunk = fn type, data ->
      <<byte_size(data)::32>> <> type <> data <> <<:erlang.crc32(type <> data)::32>>
    end

    <<137, 80, 78, 71, 13, 10, 26, 10>> <>
      chunk.("IHDR", <<w::32, h::32, 8, 6, 0, 0, 0>>) <>
      chunk.("IDAT", :zlib.compress(rows)) <> chunk.("IEND", <<>>)
  end

  setup do
    root = Path.join(System.tmp_dir!(), "gvt-ex-#{System.unique_integer([:positive])}")
    on_exit(fn -> File.rm_rf(root) end)
    [opts: [cli: @cli, dir: Path.join(root, "out"), baseline_dir: Path.join(root, "base")]]
  end

  test "expectations stay pending with regions", %{opts: opts} do
    assert {:pending, result} =
             GpuiVisualTest.check(
               png(40, 20),
               "counter/initial",
               [
                 expect: ["The count reads 0"],
                 expect_in: [label: "Reads Count"],
                 regions: [label: {0, 0, 10, 5}],
                 scale: 2
               ] ++ opts
             )

    assert result["scale"] == 2.0
    assert [%{"region" => "label"}] = Enum.filter(result["checks"], & &1["region"])

    error =
      assert_raise ExUnit.AssertionError, fn ->
        assert_visual(png(40, 20), "counter/initial", [expect: ["x"]] ++ opts)
      end

    assert error.message =~ "cargo visual-test verdict"
  end

  test "pixel diff round trip", %{opts: opts} do
    opts = [diff: 0] ++ opts

    assert {:pending, %{"diff" => %{"status" => "unsure"}}} =
             GpuiVisualTest.check(png(40, 40), "box", opts)

    assert {:ok, _} = GpuiVisualTest.check(png(40, 40), "box", [update_baseline: true] ++ opts)
    assert %{"status" => "pass"} = assert_visual(png(40, 40), "box", opts)

    error =
      assert_raise ExUnit.AssertionError, fn ->
        assert_visual(png(40, 40, {4, 4, 6}), "box", opts)
      end

    assert error.message =~ "36 of 1600 pixels changed"
  end

  test "files and errors", %{opts: opts} do
    path = Path.join(System.tmp_dir!(), "gvt-ex-file-#{System.unique_integer([:positive])}.png")
    File.write!(path, png(8, 8))
    assert {:pending, _} = GpuiVisualTest.check({:file, path}, "file", [expect: ["x"]] ++ opts)

    assert {:error, msg} =
             GpuiVisualTest.check(png(8, 8), "x", Keyword.put(opts, :cli, "/nonexistent/cvt"))

    assert msg =~ "GPUI_VISUAL_TEST_CLI"
    assert {:error, _} = GpuiVisualTest.check(png(8, 8), "", opts)
    assert {:error, _} = GpuiVisualTest.check(png(8, 8), "no-expectations", opts)
  end
end
