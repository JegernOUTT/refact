package com.smallcloud.refactai.lsp

import java.nio.file.Files
import java.nio.file.Path
import java.util.Base64
import java.util.concurrent.TimeUnit

/**
 * Registers `<home>/.refact/bin` on the user's PATH so that the shared refact binary is available
 * from a terminal, mirroring the behaviour of the standalone `install.sh` and `install.ps1`.
 *
 * The logic is deliberately side-effect free with respect to the plugin lifecycle: every entry point
 * returns a structured [RegistrationResult] and never throws, so registration can be invoked from the
 * daemon launch path without risking a crash.
 */

/** Marker written at the top of an owned block in Unix shell profiles (Codex-style). */
internal const val REFACT_PATH_MARKER_BEGIN = "# >>> Refact installer >>>"

/** Marker written at the bottom of an owned block in Unix shell profiles (Codex-style). */
internal const val REFACT_PATH_MARKER_END = "# <<< Refact installer <<<"

/** Structured, non-throwing outcome of a PATH registration attempt. */
sealed class RegistrationResult {
    /** Registration wrote or refreshed the owned block / registry value. */
    data class Changed(val detail: String) : RegistrationResult()

    /** Registration found the owned content already up to date and did nothing. */
    data class Unchanged(val detail: String) : RegistrationResult()

    /** Registration was skipped because it is not applicable (e.g. unknown platform, no home). */
    data class Skipped(val detail: String) : RegistrationResult()

    /** Registration could not be completed; [message] explains why. Never surfaced as an exception. */
    data class Warning(val message: String) : RegistrationResult()
}

/**
 * Abstraction over the Windows PATH mutation so tests never touch the real registry.
 * The default implementation invokes the built-in PowerShell through [ProcessBuilder].
 */
interface WindowsPathCommandRunner {
    /**
     * Persists [binDir] into the HKCU user PATH.
     * @return an exit code where 0 means success and anything else is treated as a failure.
     */
    fun addToUserPath(binDir: Path): WindowsPathCommandResult
}

data class WindowsPathCommandResult(val exitCode: Int, val changed: Boolean = false)

/**
 * Distinguishes the supported shells so the correct startup profile can be chosen.
 * The selection mirrors the Codex installer conventions requested by the plan.
 */
internal enum class ShellKind { ZSH, BASH, FISH, OTHER }

object RefactPathRegistrar {

    /**
     * Registers `<homeDir>/.refact/bin` for terminal use.
     *
     * On Unix a single Codex-style profile file is chosen and an owned marker block is maintained
     * idempotently. On Windows the [windowsRunner] is invoked to persist the directory into the
     * HKCU user PATH. All failures are captured as [RegistrationResult.Warning]; this method never
     * throws.
     */
    fun register(
        homeDir: Path,
        osName: String = System.getProperty("os.name"),
        shellPath: String? = System.getenv("SHELL"),
        windowsRunner: WindowsPathCommandRunner = PowerShellWindowsPathCommandRunner,
        xdgConfigHome: Path? = System.getenv("XDG_CONFIG_HOME")?.takeIf { it.isNotBlank() }?.let(Path::of),
    ): RegistrationResult {
        return try {
            val binDir = homeDir.resolve(".refact").resolve("bin").toAbsolutePath().normalize()
            if (osName.lowercase().contains("win")) {
                registerWindows(binDir, windowsRunner)
            } else {
                registerUnix(homeDir, osName, shellPath, xdgConfigHome)
            }
        } catch (e: Exception) {
            RegistrationResult.Warning("Failed to register refact PATH: ${e.message}")
        }
    }

    // --------------------------------------------------------------------------------------------
    // Unix
    // --------------------------------------------------------------------------------------------

    internal fun registerUnix(
        homeDir: Path,
        osName: String,
        shellPath: String?,
        xdgConfigHome: Path?,
    ): RegistrationResult {
        val kind = shellKind(shellPath)
        val profile = unixProfileFile(homeDir, osName, kind, xdgConfigHome)
        val exportLine = exportLine(kind)
        val ownedBlock = ownedBlock(exportLine)

        return try {
            val existing = if (Files.exists(profile)) Files.readString(profile) else ""
            val block = extractOwnedBlock(existing)
            when {
                block == null -> {
                    val updated = appendBlock(existing, ownedBlock)
                    writeProfile(profile, updated)
                    RegistrationResult.Changed("Added refact PATH block to $profile")
                }
                block.malformed -> {
                    RegistrationResult.Warning(
                        "Refactai PATH markers in $profile are malformed; leaving file untouched"
                    )
                }
                block.body.trim() == ownedBlock.trim() -> {
                    RegistrationResult.Unchanged("refact PATH block already present in $profile")
                }
                else -> {
                    val updated = replaceOwnedBlock(existing, ownedBlock)
                    writeProfile(profile, updated)
                    RegistrationResult.Changed("Updated refact PATH block in $profile")
                }
            }
        } catch (e: Exception) {
            RegistrationResult.Warning("Failed to update $profile: ${e.message}")
        }
    }

    private fun writeProfile(profile: Path, content: String) {
        profile.parent?.let { Files.createDirectories(it) }
        Files.writeString(profile, content)
    }

    /** Selects a single Codex-style startup profile for the detected OS + shell. */
    internal fun unixProfileFile(homeDir: Path, osName: String, kind: ShellKind, xdgConfigHome: Path? = null): Path {
        val macos = osName.lowercase().contains("mac") || osName.lowercase().contains("darwin")
        return when (kind) {
            ShellKind.FISH -> (xdgConfigHome ?: homeDir.resolve(".config")).resolve("fish").resolve("config.fish")
            ShellKind.ZSH -> if (macos) homeDir.resolve(".zprofile") else homeDir.resolve(".zshrc")
            ShellKind.BASH -> if (macos) homeDir.resolve(".bash_profile") else homeDir.resolve(".bashrc")
            ShellKind.OTHER -> homeDir.resolve(".profile")
        }
    }

    internal fun shellKind(shellPath: String?): ShellKind {
        val name = shellPath?.substringAfterLast('/')?.lowercase().orEmpty()
        return when {
            name.contains("zsh") -> ShellKind.ZSH
            name.contains("bash") -> ShellKind.BASH
            name.contains("fish") -> ShellKind.FISH
            else -> ShellKind.OTHER
        }
    }

    private fun exportLine(kind: ShellKind): String {
        return if (kind == ShellKind.FISH) {
            "fish_add_path \"\$HOME/.refact/bin\""
        } else {
            "export PATH=\"\$HOME/.refact/bin:\$PATH\""
        }
    }

    private fun ownedBlock(exportLine: String): String {
        return "$REFACT_PATH_MARKER_BEGIN\n$exportLine\n$REFACT_PATH_MARKER_END"
    }

    /** Result of scanning a profile for the owned marker block. */
    internal data class OwnedBlock(val body: String, val malformed: Boolean)

    /**
     * Extracts the owned marker block from [content].
     * @return null if no marker is present, an [OwnedBlock] otherwise (with [OwnedBlock.malformed]
     * set when markers are unbalanced or out of order).
     */
    internal fun extractOwnedBlock(content: String): OwnedBlock? {
        val lines = content.split("\n")
        val begins = lines.indices.filter { lines[it].removeSuffix("\r") == REFACT_PATH_MARKER_BEGIN }
        val ends = lines.indices.filter { lines[it].removeSuffix("\r") == REFACT_PATH_MARKER_END }
        if (begins.isEmpty() && ends.isEmpty()) return null
        if (begins.size != 1 || ends.size != 1 || ends.first() < begins.first()) {
            return OwnedBlock("", malformed = true)
        }
        val body = lines.subList(begins.first(), ends.first() + 1).joinToString("\n")
        return OwnedBlock(body, malformed = false)
    }

    private fun appendBlock(content: String, block: String): String {
        if (content.isEmpty()) return "$block\n"
        val separator = if (content.endsWith("\n")) "" else "\n"
        return "$content$separator\n$block\n"
    }

    private fun replaceOwnedBlock(content: String, block: String): String {
        val lines = content.split("\n")
        val begin = lines.indexOfFirst { it.removeSuffix("\r") == REFACT_PATH_MARKER_BEGIN }
        val end = lines.indexOfFirst { it.removeSuffix("\r") == REFACT_PATH_MARKER_END }
        val before = lines.subList(0, begin)
        val after = lines.subList(end + 1, lines.size)
        val rebuilt = (before + block.split("\n") + after).joinToString("\n")
        return rebuilt
    }

    // --------------------------------------------------------------------------------------------
    // Windows
    // --------------------------------------------------------------------------------------------

    internal fun registerWindows(
        binDir: Path,
        runner: WindowsPathCommandRunner,
    ): RegistrationResult {
        return try {
            val result = runner.addToUserPath(binDir)
            if (result.exitCode == 0 && result.changed) {
                RegistrationResult.Changed("Added $binDir to the Windows user PATH")
            } else if (result.exitCode == 0) {
                RegistrationResult.Unchanged("$binDir is already on the Windows user PATH")
            } else {
                RegistrationResult.Warning("PowerShell exited with code ${result.exitCode} while updating the user PATH")
            }
        } catch (e: Exception) {
            RegistrationResult.Warning("Failed to update the Windows user PATH: ${e.message}")
        }
    }
}

/**
 * Default [WindowsPathCommandRunner] that shells out to the built-in PowerShell using [ProcessBuilder].
 * The PowerShell script is idempotent and performs a case-insensitive comparison so that repeated
 * launches never duplicate the entry. No third-party dependency is required.
 */
object PowerShellWindowsPathCommandRunner : WindowsPathCommandRunner {
    override fun addToUserPath(binDir: Path): WindowsPathCommandResult {
        val script = buildPowerShellScript()
        val process = ProcessBuilder(
            "powershell.exe",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        )
            .redirectErrorStream(true)
            .start()
        val encodedDir = Base64.getEncoder().encodeToString(binDir.toString().toByteArray(Charsets.UTF_8))
        process.outputStream.bufferedWriter().use { it.write(encodedDir) }
        if (!process.waitFor(30, TimeUnit.SECONDS)) {
            process.destroyForcibly()
            return WindowsPathCommandResult(124)
        }
        val output = process.inputStream.readBytes().toString(Charsets.UTF_8)
        return WindowsPathCommandResult(process.exitValue(), output.contains("ADDED"))
    }

    /**
     * Builds a case-insensitive, idempotent PowerShell snippet that appends [dir] to the HKCU PATH
     * only when it is not already present.
     */
    internal fun buildPowerShellScript(): String {
        return """
            ${'$'}target = 'User'
            ${'$'}encodedDir = [Console]::In.ReadToEnd()
            ${'$'}dir = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String(${'$'}encodedDir))
            if ([string]::IsNullOrWhiteSpace(${'$'}dir)) { throw 'Refact bin directory is empty' }
            ${'$'}current = [Environment]::GetEnvironmentVariable('Path', ${'$'}target)
            if (${'$'}null -eq ${'$'}current) { ${'$'}current = '' }
            ${'$'}entries = ${'$'}current -split ';' | Where-Object { ${'$'}_ -ne '' }
            ${'$'}exists = ${'$'}false
            foreach (${'$'}e in ${'$'}entries) {
                if (${'$'}e.TrimEnd('\') -ieq ${'$'}dir.TrimEnd('\')) { ${'$'}exists = ${'$'}true }
            }
            if (${'$'}exists) { Write-Output 'UNCHANGED'; exit 0 }
            if (-not ${'$'}exists) {
                ${'$'}updated = if (${'$'}current -eq '') { ${'$'}dir } else { ${'$'}current.TrimEnd(';') + ';' + ${'$'}dir }
                [Environment]::SetEnvironmentVariable('Path', ${'$'}updated, ${'$'}target)
                Write-Output 'ADDED'
            }
        """.trimIndent()
    }
}
