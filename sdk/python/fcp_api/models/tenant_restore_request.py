from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.file_item import FileItem





T = TypeVar("T", bound="TenantRestoreRequest")



@_attrs_define
class TenantRestoreRequest:
    """ Lost files to put back from one backup.

        Attributes:
            point (str): A `held_in` point from the missing list.
            items (list[FileItem]):
     """

    point: str
    items: list[FileItem]





    def to_dict(self) -> dict[str, Any]:
        from ..models.file_item import FileItem # noqa: PLC0415
        point = self.point

        items = []
        for items_item_data in self.items:
            items_item = items_item_data.to_dict()
            items.append(items_item)




        field_dict: dict[str, Any] = {}

        field_dict.update({
            "point": point,
            "items": items,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.file_item import FileItem # noqa: PLC0415
        d = dict(src_dict)
        point = d.pop("point")

        items = []
        _items = d.pop("items")
        for items_item_data in (_items):
            items_item = FileItem.from_dict(items_item_data)



            items.append(items_item)


        tenant_restore_request = cls(
            point=point,
            items=items,
        )

        return tenant_restore_request

