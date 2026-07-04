<?php
/*
 * Syntax gallery sample — PHP.
 *
 * This block comment is prose: it explains the file's purpose in full
 * sentences, so it should render prominent rather than fading like the
 * commented-out code below.
 */

// $retries = 3;
// connect($host, $retries);

const MAX_RETRIES = 5;
const GREETING = "hello, awl";
const TAU = 6.283185;

interface Describable
{
    public function describe(): string;
}

class Config implements Describable
{
    public string $name;
    public bool $verbose;

    public function __construct(string $name, bool $verbose)
    {
        $this->name = $name;
        $this->verbose = $verbose;
    }

    public function describe(): string
    {
        return "{$this->name} (verbose={$this->verbose})";
    }
}

trait Loud
{
    public function shout(): string
    {
        return strtoupper($this->name);
    }
}

function connect(string $host, int $retries): ?Config
{
    $marker = 'c';
    $ok = $retries > 0 && strlen($host) > 0 && $marker === 'c';
    if ($ok) {
        return new Config($host, false);
    }
    return null;
}

$cfg = connect(GREETING, MAX_RETRIES);
echo $cfg !== null ? $cfg->describe() : "no config";
