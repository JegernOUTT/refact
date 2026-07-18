package com.smallcloud.refactai.panes.sharedchat

import com.intellij.openapi.application.PathManager
import java.io.RandomAccessFile
import java.text.SimpleDateFormat
import java.util.Locale

data class IdeLogLine(val at: Long?, val level: String, val message: String)

object IdeaLogReader {
    private const val TAIL_BYTES = 262144L
    private const val MAX_SCANNED_LINES = 10000
    private val headerRegex = Regex("""^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3})\s+\[\s*\d+\]\s+(ERROR|WARN|INFO|DEBUG|TRACE|SEVERE)\s+-\s+(\S+)\s+-\s?(.*)$""")

    fun collectRefactLogLines(limit: Int): List<IdeLogLine> {
        return runCatching {
            val logFile = java.io.File(PathManager.getLogPath(), "idea.log")
            if (!logFile.isFile) return emptyList()

            RandomAccessFile(logFile, "r").use { file ->
                val length = file.length()
                val start = (length - TAIL_BYTES).coerceAtLeast(0)
                file.seek(start)
                val bytes = ByteArray((length - start).coerceAtMost(TAIL_BYTES).toInt())
                file.readFully(bytes)
                val text = bytes.toString(Charsets.UTF_8)
                val lines = text.lineSequence().toMutableList()
                if (start > 0 && lines.isNotEmpty()) {
                    lines.removeAt(0)
                }
                parseAndFilter(lines, limit)
            }
        }.getOrDefault(emptyList())
    }

    fun parseAndFilter(lines: List<String>, limit: Int): List<IdeLogLine> {
        val boundedLimit = limit.coerceIn(1, 500)
        val kept = ArrayList<IdeLogLine>()
        var keepContinuations = false
        var continuationLevel = "info"

        lines.takeLast(MAX_SCANNED_LINES).forEach { line ->
            val match = headerRegex.matchEntire(line)
            if (match != null) {
                val timestamp = match.groupValues[1]
                val rawLevel = match.groupValues[2]
                val category = match.groupValues[3]
                val message = match.groupValues[4]
                val keep = isRefactCategory(category)

                keepContinuations = keep
                continuationLevel = mapLevel(rawLevel)

                if (keep) {
                    kept.add(IdeLogLine(parseTimestamp(timestamp), continuationLevel, "$category - $message"))
                }
            } else if (keepContinuations) {
                kept.add(IdeLogLine(null, continuationLevel, line))
            }
        }

        return kept.takeLast(boundedLimit)
    }

    private fun isRefactCategory(category: String): Boolean {
        val lower = category.lowercase(Locale.ROOT)
        return lower.contains("smallcloud") || lower.contains("refactai") || lower.startsWith("#c.s.r")
    }

    private fun mapLevel(level: String): String {
        return when (level) {
            "ERROR", "SEVERE" -> "error"
            "WARN" -> "warn"
            "DEBUG", "TRACE" -> "debug"
            else -> "info"
        }
    }

    private fun parseTimestamp(timestamp: String): Long? {
        return runCatching {
            SimpleDateFormat("yyyy-MM-dd HH:mm:ss,SSS").parse(timestamp)?.time
        }.getOrNull()
    }
}
