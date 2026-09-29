from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.audit_entry_response_new_value import AuditEntryResponseNewValue
  from ..models.audit_entry_response_old_value_type_0 import AuditEntryResponseOldValueType0





T = TypeVar("T", bound="AuditEntryResponse")



@_attrs_define
class AuditEntryResponse:
    """ 
        Attributes:
            id (int):
            service (str):
            section (str):
            tenant_id (str):
            new_value (AuditEntryResponseNewValue):
            changed_at (str):
            version (int):
            old_value (AuditEntryResponseOldValueType0 | None | Unset):
            changed_by (None | str | Unset):
     """

    id: int
    service: str
    section: str
    tenant_id: str
    new_value: AuditEntryResponseNewValue
    changed_at: str
    version: int
    old_value: AuditEntryResponseOldValueType0 | None | Unset = UNSET
    changed_by: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.audit_entry_response_new_value import AuditEntryResponseNewValue # noqa: PLC0415
        from ..models.audit_entry_response_old_value_type_0 import AuditEntryResponseOldValueType0 # noqa: PLC0415
        id = self.id

        service = self.service

        section = self.section

        tenant_id = self.tenant_id

        new_value = self.new_value.to_dict()

        changed_at = self.changed_at

        version = self.version

        old_value: dict[str, Any] | None | Unset
        if isinstance(self.old_value, Unset):
            old_value = UNSET
        elif isinstance(self.old_value, AuditEntryResponseOldValueType0):
            old_value = self.old_value.to_dict()
        else:
            old_value = self.old_value

        changed_by: None | str | Unset
        if isinstance(self.changed_by, Unset):
            changed_by = UNSET
        else:
            changed_by = self.changed_by


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "service": service,
            "section": section,
            "tenant_id": tenant_id,
            "new_value": new_value,
            "changed_at": changed_at,
            "version": version,
        })
        if old_value is not UNSET:
            field_dict["old_value"] = old_value
        if changed_by is not UNSET:
            field_dict["changed_by"] = changed_by

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.audit_entry_response_new_value import AuditEntryResponseNewValue # noqa: PLC0415
        from ..models.audit_entry_response_old_value_type_0 import AuditEntryResponseOldValueType0 # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        service = d.pop("service")

        section = d.pop("section")

        tenant_id = d.pop("tenant_id")

        new_value = AuditEntryResponseNewValue.from_dict(d.pop("new_value"))




        changed_at = d.pop("changed_at")

        version = d.pop("version")

        def _parse_old_value(data: object) -> AuditEntryResponseOldValueType0 | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                old_value_type_0 = AuditEntryResponseOldValueType0.from_dict(data)



                return old_value_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(AuditEntryResponseOldValueType0 | None | Unset, data)

        old_value = _parse_old_value(d.pop("old_value", UNSET))


        def _parse_changed_by(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        changed_by = _parse_changed_by(d.pop("changed_by", UNSET))


        audit_entry_response = cls(
            id=id,
            service=service,
            section=section,
            tenant_id=tenant_id,
            new_value=new_value,
            changed_at=changed_at,
            version=version,
            old_value=old_value,
            changed_by=changed_by,
        )


        audit_entry_response.additional_properties = d
        return audit_entry_response

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
