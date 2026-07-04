# Syntax gallery sample — Ruby.
#
# Prose comment first: it reads as an explanation, not code, so it renders
# prominent rather than fading like the disabled code below.

# retries = 3;

MAX_RETRIES = 5
GREETING = "hello, awl"
TAU = 6.283185

class Config
  attr_reader :name, :verbose

  def initialize(name, verbose)
    @name = name
    @verbose = verbose
  end

  def describe
    "#{@name} (verbose=#{@verbose})"
  end

  def valid?
    !@name.empty?
  end
end

def connect(host, retries)
  marker = ?c
  ok = retries > 0 && !host.empty? && marker == 'c'
  if ok
    Config.new(host, false)
  else
    nil
  end
end

module Util
  def self.loud(text)
    text.upcase
  end
end

cfg = connect(GREETING, MAX_RETRIES)
if cfg
  puts cfg.describe
else
  puts "no config, was #{nil}"
end
