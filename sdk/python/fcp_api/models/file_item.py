from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="FileItem")



@_attrs_define
class FileItem:
    """ One file named by its row.

        Attributes:
            class_ (str):
            item (str):
     """

    class_: str
    item: str





    def to_dict(self) -> dict[str, Any]:
        class_ = self.class_

        item = self.item


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "class": class_,
            "item": item,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        class_ = d.pop("class")

        item = d.pop("item")

        file_item = cls(
            class_=class_,
            item=item,
        )

        return file_item

