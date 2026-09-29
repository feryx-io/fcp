from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.update_config_request_value import UpdateConfigRequestValue





T = TypeVar("T", bound="UpdateConfigRequest")



@_attrs_define
class UpdateConfigRequest:
    """ 
        Attributes:
            value (UpdateConfigRequestValue): A configuration section: a JSON object.
            tenant_id (str | Unset):
            updated_by (None | str | Unset):
     """

    value: UpdateConfigRequestValue
    tenant_id: str | Unset = UNSET
    updated_by: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.update_config_request_value import UpdateConfigRequestValue # noqa: PLC0415
        value = self.value.to_dict()

        tenant_id = self.tenant_id

        updated_by: None | str | Unset
        if isinstance(self.updated_by, Unset):
            updated_by = UNSET
        else:
            updated_by = self.updated_by


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "value": value,
        })
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id
        if updated_by is not UNSET:
            field_dict["updated_by"] = updated_by

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.update_config_request_value import UpdateConfigRequestValue # noqa: PLC0415
        d = dict(src_dict)
        value = UpdateConfigRequestValue.from_dict(d.pop("value"))




        tenant_id = d.pop("tenant_id", UNSET)

        def _parse_updated_by(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        updated_by = _parse_updated_by(d.pop("updated_by", UNSET))


        update_config_request = cls(
            value=value,
            tenant_id=tenant_id,
            updated_by=updated_by,
        )


        update_config_request.additional_properties = d
        return update_config_request

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
