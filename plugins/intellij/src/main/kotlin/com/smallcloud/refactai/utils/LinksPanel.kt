package com.smallcloud.refactai.utils

import com.intellij.icons.AllIcons
import com.intellij.ide.BrowserUtil
import com.intellij.ui.components.labels.LinkLabel
import java.awt.BorderLayout
import javax.swing.JPanel

fun makeLinksPanel(): JPanel {
    return JPanel(BorderLayout()).apply {
        add(LinkLabel<String>("Bug report", AllIcons.Ide.External_link_arrow).apply {

            setListener({ _, _ ->
                BrowserUtil.browse("https://github.com/JegernOUTT/refact/issues/new?labels=bug%2Cplugin%3Aintellij")
            }, null)
        }, BorderLayout.WEST)
    }
}
