package org.openmyglass.android.notifications

/**
 * Turns a notification's already-extracted values into a JSON-able map for the capture tool.
 * Pure Kotlin so it can be unit-tested; image data never appears in the output (clean-room rule:
 * Google bitmaps are not committed), only metadata produced by the caller.
 */
object NotificationDump {

    fun dump(
        packageName: String,
        postTime: Long,
        category: String?,
        isOngoing: Boolean,
        extras: Map<String, Any?>,
        actionTitles: List<String>,
        hasCustomContentView: Boolean,
        hasCustomBigView: Boolean,
    ): Map<String, Any?> = linkedMapOf(
        "package" to packageName,
        "postTime" to postTime,
        "category" to category,
        "ongoing" to isOngoing,
        "actions" to actionTitles,
        "hasCustomContentView" to hasCustomContentView,
        "hasCustomBigView" to hasCustomBigView,
        "extras" to extras.mapValues { (_, v) -> normalize(v) },
    )

    private fun normalize(value: Any?): Any? = when (value) {
        null, is String, is Number, is Boolean -> value
        is CharSequence -> value.toString()
        is Map<*, *> -> value.entries.associate { (k, v) -> k.toString() to normalize(v) }
        is List<*> -> value.map(::normalize)
        is Array<*> -> value.map(::normalize)
        else -> mapOf("type" to value::class.java.simpleName)
    }

    fun toJson(value: Any?): String = StringBuilder().also { appendJson(it, value) }.toString()

    private fun appendJson(sb: StringBuilder, value: Any?) {
        when (value) {
            null -> sb.append("null")
            is Boolean, is Int, is Long, is Short, is Byte -> sb.append(value)
            is Number -> sb.append(value.toDouble().let { if (it.isFinite()) it.toString() else "null" })
            is Map<*, *> -> {
                sb.append('{')
                value.entries.forEachIndexed { i, (k, v) ->
                    if (i > 0) sb.append(',')
                    appendString(sb, k.toString()); sb.append(':'); appendJson(sb, v)
                }
                sb.append('}')
            }
            is List<*> -> {
                sb.append('[')
                value.forEachIndexed { i, v -> if (i > 0) sb.append(','); appendJson(sb, v) }
                sb.append(']')
            }
            else -> appendString(sb, value.toString())
        }
    }

    private fun appendString(sb: StringBuilder, s: String) {
        sb.append('"')
        for (c in s) when (c) {
            '"' -> sb.append("\\\"")
            '\\' -> sb.append("\\\\")
            '\n' -> sb.append("\\n")
            '\r' -> sb.append("\\r")
            '\t' -> sb.append("\\t")
            else -> if (c < ' ') sb.append("\\u%04x".format(c.code)) else sb.append(c)
        }
        sb.append('"')
    }
}
