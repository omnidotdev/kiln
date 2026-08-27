# A Sinatra app started via `ruby main.rb` (the provider's non-Rails start).
# Exercises: gems installed into the image layer (require "sinatra" must resolve
# at runtime), and a lockfile-less bundle install.
require "sinatra"

set :bind, "0.0.0.0"
set :port, (ENV["PORT"] || 3000).to_i
set :server, "puma"

get "/" do
  "ok\n"
end
