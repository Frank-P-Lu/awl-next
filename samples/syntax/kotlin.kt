/*
 * Syntax gallery sample — Kotlin.
 *
 * This block comment is prose: it explains the file's purpose in full
 * sentences, so it should render prominent rather than fading like the
 * commented-out code below.
 */

// val retries = 3;
// connect(host, retries);

const val MAX_RETRIES = 5
const val GREETING = "hello, awl"
const val TAU = 6.283185
val marker = 'c'

typealias Retries = Int

data class Config(val name: String, val verbose: Boolean)

interface Describable {
    fun describe(): String
}

object Defaults {
    val verbose = false
}

class Connection(private val config: Config) : Describable {
    override fun describe(): String {
        return "${config.name} (verbose=${config.verbose})"
    }
}

fun connect(host: String, retries: Retries): Config? {
    val ok = retries > 0 && host.isNotEmpty() && marker == 'c'
    return if (ok) Config(host, Defaults.verbose) else null
}

fun main() {
    val cfg = connect(GREETING, MAX_RETRIES)
    val fallback: Config? = null
    if (cfg != null) {
        println(Connection(cfg).describe())
    } else {
        println("no config, fallback is $fallback")
    }
}
