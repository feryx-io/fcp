from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.tenant_restored import TenantRestored





T = TypeVar("T", bound="TenantRestoreOutcome")



@_attrs_define
class TenantRestoreOutcome:
    """ What a tenant's restore put back.

        Attributes:
            restored (list[TenantRestored]):
     """

    restored: list[TenantRestored]
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.tenant_restored import TenantRestored # noqa: PLC0415
        restored = []
        for restored_item_data in self.restored:
            restored_item = restored_item_data.to_dict()
            restored.append(restored_item)




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "restored": restored,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.tenant_restored import TenantRestored # noqa: PLC0415
        d = dict(src_dict)
        restored = []
        _restored = d.pop("restored")
        for restored_item_data in (_restored):
            restored_item = TenantRestored.from_dict(restored_item_data)



            restored.append(restored_item)


        tenant_restore_outcome = cls(
            restored=restored,
        )


        tenant_restore_outcome.additional_properties = d
        return tenant_restore_outcome

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
