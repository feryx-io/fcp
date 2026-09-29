from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast






T = TypeVar("T", bound="ReencryptReport")



@_attrs_define
class ReencryptReport:
    """ Outcome of a bulk re-seal pass.

        Attributes:
            rewritten (int): Rows rewritten with the active DEK (were plaintext or an older DEK).
            already_current (int): Rows already sealed with the active DEK.
            by_column (list[list[Any]]): Per-column breakdown of rewritten rows (`table.column` → count).
                `[column, rows]` pairs.
     """

    rewritten: int
    already_current: int
    by_column: list[list[Any]]
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        rewritten = self.rewritten

        already_current = self.already_current

        by_column = []
        for by_column_item_data in self.by_column:
            by_column_item = by_column_item_data


            by_column.append(by_column_item)




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "rewritten": rewritten,
            "already_current": already_current,
            "by_column": by_column,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        rewritten = d.pop("rewritten")

        already_current = d.pop("already_current")

        by_column = []
        _by_column = d.pop("by_column")
        for by_column_item_data in (_by_column):
            by_column_item = cast(list[Any], by_column_item_data)

            by_column.append(by_column_item)


        reencrypt_report = cls(
            rewritten=rewritten,
            already_current=already_current,
            by_column=by_column,
        )


        reencrypt_report.additional_properties = d
        return reencrypt_report

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
