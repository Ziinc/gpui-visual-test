defmodule GpuiVisualTest.Assertions do
  @moduledoc """
  ExUnit assertions for visual checks.

      import GpuiVisualTest.Assertions

      test "counter starts at zero" do
        png = render_png(MyApp.Counter)
        assert_visual png, "counter/initial",
          expect: ["The count reads 0"],
          diff: 0.001
      end

  A pending check (no model judge, or no approved baseline yet) fails the
  test with the commands that settle it, unless `GPUI_VISUAL_UNSURE=warn`.
  """

  @doc "Fails unless the visual check passes; returns the result map."
  defmacro assert_visual(image, name, opts \\ []) do
    quote do
      GpuiVisualTest.Assertions.__assert__(unquote(image), unquote(name), unquote(opts))
    end
  end

  @doc false
  def __assert__(image, name, opts) do
    case GpuiVisualTest.check(image, name, opts) do
      {:ok, result} ->
        result

      {status, result} when status in [:fail, :pending] ->
        raise ExUnit.AssertionError, message: GpuiVisualTest.describe(result)

      {:error, message} ->
        raise ExUnit.AssertionError, message: message
    end
  end
end
