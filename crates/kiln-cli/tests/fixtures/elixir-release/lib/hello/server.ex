defmodule Hello.Server do
  @moduledoc "A minimal raw-socket HTTP server, so the release has no hex deps."
  use Task, restart: :permanent

  def start_link(port), do: Task.start_link(fn -> listen(port) end)

  defp listen(port) do
    {:ok, sock} =
      :gen_tcp.listen(port, [
        :binary,
        packet: :line,
        active: false,
        reuseaddr: true,
        ip: {0, 0, 0, 0}
      ])

    accept(sock)
  end

  defp accept(sock) do
    {:ok, client} = :gen_tcp.accept(sock)
    spawn(fn -> handle(client) end)
    accept(sock)
  end

  defp handle(client) do
    _ = :gen_tcp.recv(client, 0)
    :gen_tcp.send(client, "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nok\n")
    :gen_tcp.close(client)
  end
end
