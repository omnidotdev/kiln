// Minimal raw-socket HTTP server. Exercises the C++/CMake path: cmake is
// provisioned on gcc:14, the binary is copied to /app/app and launched via
// ./app (not ./build/app), and the debian-slim runtime has libstdc++.
#include <cstdlib>
#include <iostream>
#include <string>
#include <netinet/in.h>
#include <unistd.h>

int main() {
  const char* env = std::getenv("PORT");
  int port = env ? std::atoi(env) : 8080;

  int fd = socket(AF_INET, SOCK_STREAM, 0);
  int opt = 1;
  setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));

  sockaddr_in addr{};
  addr.sin_family = AF_INET;
  addr.sin_addr.s_addr = INADDR_ANY;
  addr.sin_port = htons(static_cast<uint16_t>(port));
  if (bind(fd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) < 0) {
    std::cerr << "bind failed\n";
    return 1;
  }
  listen(fd, 16);
  std::cout << "listening on 0.0.0.0:" << port << std::endl;

  const std::string resp = "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nok\n";
  for (;;) {
    int c = accept(fd, nullptr, nullptr);
    if (c < 0) continue;
    char buf[1024];
    read(c, buf, sizeof(buf));
    write(c, resp.data(), resp.size());
    close(c);
  }
}
