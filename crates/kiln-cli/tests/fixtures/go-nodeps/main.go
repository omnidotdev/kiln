// A dependency-free Go web server. Exercises the go.sum-optional build (this
// module has no go.sum), CGO_ENABLED=0, the distroless/static runtime, and the
// exec-form CMD (distroless has no shell for a `sh -c` wrapper).
package main

import (
	"fmt"
	"log"
	"net/http"
	"os"
)

func main() {
	http.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprintln(w, "ok")
	})
	port := os.Getenv("PORT")
	if port == "" {
		port = "8080"
	}
	log.Printf("listening on 0.0.0.0:%s", port)
	log.Fatal(http.ListenAndServe("0.0.0.0:"+port, nil))
}
