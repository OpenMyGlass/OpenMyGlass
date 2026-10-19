package org.openmyglass.simglass

import kotlin.system.exitProcess

/** `sim-glass [--port N] [--silent] [--battery N] [--bridge]` — runs until interrupted. `--bridge` simulates our bridge app instead of the firmware companion server. */
fun main(args: Array<String>) {
    var config = SimGlassConfig(port = 7823)
    var bridge = false
    var portSet = false
    var i = 0
    while (i < args.size) {
        when (args[i]) {
            "--port" -> { config = config.copy(port = args.getOrNull(++i)?.toIntOrNull() ?: usage()); portSet = true }
            "--bridge" -> bridge = true
            "--silent" -> config = config.copy(silent = true)
            "--battery" -> config = config.copy(info = config.info.copy(batteryLevel = args.getOrNull(++i)?.toIntOrNull() ?: usage()))
            else -> usage()
        }
        i++
    }
    val sim: java.io.Closeable = if (bridge) {
        SimBridge(if (portSet) config.port else 7824) { println(it) }.also { it.start() }
    } else {
        SimGlass(config) { println(it) }.also { it.start() }
    }
    Runtime.getRuntime().addShutdownHook(Thread { sim.close() })
    Thread.currentThread().join()
}

private fun usage(): Nothing {
    System.err.println("usage: sim-glass [--port N] [--silent] [--battery N] [--bridge]")
    exitProcess(2)
}
