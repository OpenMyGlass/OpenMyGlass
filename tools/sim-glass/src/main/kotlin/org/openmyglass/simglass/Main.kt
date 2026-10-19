package org.openmyglass.simglass

import kotlin.system.exitProcess

/** `sim-glass [--port N] [--silent] [--battery N]` — runs until interrupted. */
fun main(args: Array<String>) {
    var config = SimGlassConfig(port = 7823)
    var i = 0
    while (i < args.size) {
        when (args[i]) {
            "--port" -> config = config.copy(port = args.getOrNull(++i)?.toIntOrNull() ?: usage())
            "--silent" -> config = config.copy(silent = true)
            "--battery" -> config = config.copy(info = config.info.copy(batteryLevel = args.getOrNull(++i)?.toIntOrNull() ?: usage()))
            else -> usage()
        }
        i++
    }
    val sim = SimGlass(config) { println(it) }
    Runtime.getRuntime().addShutdownHook(Thread { sim.close() })
    sim.start()
    Thread.currentThread().join()
}

private fun usage(): Nothing {
    System.err.println("usage: sim-glass [--port N] [--silent] [--battery N]")
    exitProcess(2)
}
