<?php
// A plain PHP app (no composer.lock). Exercises the optional-lockfile COPY and
// the apache runtime, which listens on port 80 (the plan must declare 80).
header("Content-Type: text/plain");
echo "ok\n";
