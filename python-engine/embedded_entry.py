"""Inert __main__ file for libraries inspecting the embedded Python host.

The actual application entry point is Rust. Importing this marker never starts
the GUI, loads models, or schedules work.
"""
