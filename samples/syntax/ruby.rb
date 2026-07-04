# Syntax gallery sample — Ruby.
#
# This paragraph is a prose comment: several stacked line comments that read
# as an explanation, not code, so they should render prominent rather than
# fading like the commented-out code below.

# retries = 3;
# connect(host, retries);

MAX_RETRIES = 5
GREETING = "hello, awl"
TAU = 6.283185

module Util
  def self.loud(text)
    text.upcase
  end
end

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

cfg = connect(GREETING, MAX_RETRIES)
if cfg
  puts cfg.describe
else
  puts "no config, was #{nil}"
end
